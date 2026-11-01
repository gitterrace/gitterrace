//! HTTP server: server-rendered pages over `gitterrace-git` and `gitterrace-index`.

mod about;
mod actions;
mod app;
mod charts;
mod error;
mod filter;
mod fmt;
mod gitstate;
mod guard;
mod highlight;
mod insights;
mod languages;
mod live;
mod markdown;
mod media;
mod multi;
mod pages;
mod registry;
mod reload;
mod routes;
mod search;
pub mod sitemap;
mod types;
mod views;
#[cfg(test)]
mod wiring_tests;

use std::{
    io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use gitterrace_config::Config;
use gitterrace_git::Repo;
use gitterrace_index::Index;
pub use multi::discover_repos;
pub use registry::{Entry as RegistryEntry, candidates as registry_candidates, state_dir};
pub use reload::{Load, Watched};
pub use views::stats_tab_labels;

use crate::app::{App, Env};

/// The full router `serve` uses: pages and `/api/v1`. Starts the index
/// refresher, so call it inside a tokio runtime. Drive it in-process with tower's `oneshot`.
///
/// Requests must carry a loopback `Host` header (`localhost`, `127.0.0.1`, `[::1]`), which
/// stops DNS-rebinding pages from reading the repository. In-process callers send
/// `Host: localhost`.
pub fn app_router(repo_path: impl AsRef<Path>) -> io::Result<axum::Router> {
    app_router_with(repo_path, Config::default())
}

/// [`app_router`] with the settings in force.
pub fn app_router_with(repo_path: impl AsRef<Path>, cfg: Config) -> io::Result<axum::Router> {
    Ok(build(repo_path.as_ref(), None, None, &Env::new(cfg))?.0)
}

/// Router for several repositories: an index at `/`, each repository under `/r/NAME`.
/// One path behaves exactly like [`app_router`]. Same Host rules.
pub fn multi_router(paths: Vec<PathBuf>) -> io::Result<axum::Router> {
    Ok(build_multi(paths, None, None, &Env::new(Config::default()))?.0)
}

/// One repository's pages and `/api/v1`, with links under `base`. Starts its
/// index refresher, so call it inside a tokio runtime.
fn repo_router(
    repo_path: &Path,
    base: String,
    key: String,
    live: Option<Arc<actions::Live>>,
    env: &Env,
) -> io::Result<axum::Router> {
    let cfg = env.cfg();
    let (text_bytes, blame_bytes) = app::repo_limits(&cfg);
    let repo = Repo::discover(repo_path)
        .map_err(io::Error::other)?
        .with_limits(text_bytes, blame_bytes)
        .with_workers(cfg.index.threads);
    let index = Index::open(&repo)?.with_threads(cfg.index.threads);
    let mut app = App::configured(repo, index, env);
    app.base = base;
    app.key = key;
    app.actions = live;
    let app = Arc::new(app);
    env.register(&app);
    tokio::spawn(app::keep_index_fresh(Arc::clone(&app)));
    let style = gitterrace_actions::LinkStyle::from_config(&env.cfg().links.style);
    let api = gitterrace_api::ApiState::open(repo_path)
        .map_err(io::Error::other)?
        .with_link_style(style);
    let pages = routes::router(Arc::clone(&app)).nest("/api/v1", gitterrace_api::router(api));
    tokio::spawn(prewarm(Arc::clone(&app), pages.clone()));
    Ok(pages)
}

/// Unfiltered pages rendered in the background whenever HEAD moves. Each costs 40-90 ms to
/// build on a 14k-commit repo, then a few ms from the memo, so no visitor pays the build.
/// `export` seeds its crawl from the same list, so a static copy has every insights page.
pub const WARM_PAGES: [&str; 11] = [
    "/",
    "/stats",
    "/stats/files",
    "/stats/types",
    "/stats/languages",
    "/stats/frequency",
    "/stats/contributors",
    "/stats/pulse",
    "/stats/hotspots",
    "/stats/ownership",
    "/stats/patterns",
];

/// Parses the record set for HEAD each time the index moves, then renders the unfiltered pages
/// once, so the first visit to any of them is served from the memo. Sleeps between moves.
async fn prewarm(app: Arc<App>, pages: axum::Router) {
    use tower::ServiceExt;
    let by = Arc::clone(&app);
    app::on_refreshed(app, move || {
        let (app, pages) = (Arc::clone(&by), pages.clone());
        async move {
            let ready = app::blocking(&app, |a| {
                let head = a.repo.resolve("HEAD")?;
                Ok(a.records(&head)?.is_some())
            })
            .await
            .unwrap_or(false);
            if !ready {
                return;
            }
            for path in WARM_PAGES {
                // Internal requests skip the Host and cross-site guard, which wraps the outer
                // router; a failed warm-up only means the first visitor builds the page.
                if let Ok(req) = axum::http::Request::get(path).body(axum::body::Body::empty()) {
                    let _warm = pages.clone().oneshot(req).await;
                }
            }
        }
    })
    .await;
}

fn guarded(router: axum::Router, extra: Option<IpAddr>) -> axum::Router {
    use tower_http::compression::{CompressionLayer, Predicate, predicate::NotForContentType};
    // Fonts and images are already compressed; the default predicate also skips tiny bodies.
    let worth_it = tower_http::compression::DefaultPredicate::new()
        .and(NotForContentType::new("font/"))
        .and(NotForContentType::IMAGES);
    router
        .layer(
            CompressionLayer::new()
                .gzip(true)
                .br(true)
                .compress_when(worth_it),
        )
        .layer(axum::middleware::from_fn_with_state(extra, guard::guard))
}

fn build(
    repo_path: &Path,
    extra: Option<IpAddr>,
    actions_file: Option<&Path>,
    env: &Env,
) -> io::Result<(axum::Router, String)> {
    let repo = Repo::discover(repo_path).map_err(io::Error::other)?;
    let name = repo.name();
    let (live, status) = actions::start(actions_file, &[repo.root()], extra.is_none());
    env.set_actions_status(&status);
    let sends = actions::sends(live.as_ref(), actions_file, &[repo.root()]);
    env.set_sends(&sends);
    let site = Arc::new(actions::Site {
        targets: vec![actions::Target {
            key: name.clone(),
            base: String::new(),
            repo,
        }],
        live: live.clone(),
        status,
        sends,
        env: env.clone(),
    });
    let pages = repo_router(repo_path, String::new(), name.clone(), live, env)?;
    Ok((guarded(pages.merge(actions::router(site)), extra), name))
}

fn build_multi(
    paths: Vec<PathBuf>,
    extra: Option<IpAddr>,
    actions_file: Option<&Path>,
    env: &Env,
) -> io::Result<(axum::Router, String)> {
    match <[PathBuf; 1]>::try_from(paths) {
        Ok([one]) => build(&one, extra, actions_file, env),
        Err(paths) => {
            let n = paths.len();
            let mut table = multi::Table::new(paths, env.clone())?;
            let targets = table.targets()?;
            let roots: Vec<PathBuf> = targets.iter().map(|t| t.repo.root()).collect();
            let (live, status) = actions::start(actions_file, &roots, extra.is_none());
            env.set_actions_status(&status);
            table.live.clone_from(&live);
            let sends = actions::sends(live.as_ref(), actions_file, &roots);
            env.set_sends(&sends);
            let site = Arc::new(actions::Site {
                targets,
                live,
                status,
                sends,
                env: env.clone(),
            });
            Ok((
                guarded(
                    multi::router(Arc::new(table)).merge(actions::router(site)),
                    extra,
                ),
                format!("{n} repositories"),
            ))
        }
    }
}

/// Records the served repositories for `git terrace open`. Best effort: a read-only or odd
/// state folder must not stop the server, so a failure is only logged.
fn remember(paths: &[PathBuf]) {
    let Some(dir) = registry::state_dir() else {
        return;
    };
    let repos: Vec<Repo> = paths
        .iter()
        .filter_map(|p| Repo::discover(p).ok())
        .collect();
    let refs: Vec<&Repo> = repos.iter().collect();
    if let Err(e) = registry::record(&dir, &refs) {
        tracing::warn!(
            "not recording served repositories in {}: {e}",
            dir.display()
        );
    }
}

/// Starts serving until ctrl-c. `actions` is the user's `actions.toml`; `None` (no config
/// folder, or `--no-actions`) leaves the run route unmounted, as does a non-loopback bind.
/// `watched` is the `config.toml` to reload while running, see [`Watched`].
pub async fn serve(
    repo_paths: Vec<PathBuf>,
    addr: SocketAddr,
    actions: Option<PathBuf>,
    cfg: Config,
    watched: Option<Watched>,
) -> io::Result<()> {
    let extra = (!addr.ip().is_loopback()).then(|| addr.ip());
    tokio::task::spawn_blocking({
        let paths = repo_paths.clone();
        move || remember(&paths)
    });
    let env = Env::new(cfg);
    let (router, name) = build_multi(repo_paths, extra, actions.as_deref(), &env)?;
    if let Some(w) = watched {
        tokio::spawn(reload::run(reload::Reloader::new(w), env.clone()));
    }
    if !addr.ip().is_loopback() {
        tracing::warn!(
            "serving to the network: anyone who can reach this address can read the repository"
        );
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("serving {name} on http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
            // Open tabs hold their streams; closing them lets the server stop.
            env.shut_down();
        })
        .await
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use gitterrace_testkit::{Fixture, git};
    use std::sync::Arc;

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[cfg(unix)]
    use crate::actions::actions_tests;
    use crate::{app::App, routes};

    async fn get(app: &Arc<App>, uri: &str) -> (u16, String) {
        gitterrace_testkit::get(&routes::router(Arc::clone(app)), uri).await
    }

    #[tokio::test]
    async fn about_tabs_follow_the_docs_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = Fixture::init(dir.path())
            .write("README.md", "# Hi\n")
            .write("LICENSE", "Permission <is> granted\n")
            .write("SECURITY.md", "# Report here\n")
            .write("contributing.md", "# Help\n")
            .write("docs/LICENSE/x.txt", "folder named LICENSE\n")
            .commit("feat: docs");
        let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        let (s, body) = get(&app, "/").await;
        assert_eq!(s, 200);
        assert_eq!(body.matches("class=\"tabs doctabs\"").count(), 1);
        let nav = body
            .split("tabs doctabs")
            .nth(1)
            .and_then(|t| t.split("</nav>").next())
            .unwrap_or_default();
        let labels: Vec<&str> = nav
            .split("</a>")
            .filter_map(|t| t.rsplit('>').next())
            .filter(|t| !t.is_empty())
            .collect();
        assert_eq!(
            labels,
            ["README", "LICENSE", "CONTRIBUTING", "SECURITY"],
            "{nav}"
        );
        let (_, body) = get(&app, "/?doc=license").await;
        assert!(body.contains("Permission &lt;is&gt; granted"));
        assert!(body.contains("aria-current=\"page\""));
        let (_, body) = get(&app, "/tree/HEAD/docs").await;
        assert!(
            !body.contains("doctabs"),
            "a folder called LICENSE is no doc"
        );
    }

    #[tokio::test]
    async fn readme_alone_has_no_tab_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = Fixture::standard(dir.path());
        let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        let (_, body) = get(&app, "/").await;
        assert!(body.contains("readme-h") && !body.contains("doctabs"));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn serves_every_page() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("README.md"), "# Hello <b>\n").expect("write");
        std::fs::create_dir(p.join("src")).expect("dir");
        std::fs::write(p.join("src/a.rs"), "fn main() {}\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "feat(core): first"]);
        std::fs::write(p.join("src/a.rs"), "fn main() {\n}\n").expect("write");
        git(p, &["commit", "-qam", "fix: second"]);

        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));

        let (s, body) = get(&app, "/").await;
        assert_eq!(s, 200);
        assert!(body.contains("<h1 id=\"user-content-hello"));
        let (s, body) = get(&app, "/log/HEAD").await;
        assert_eq!((s, body.contains("fix: second")), (200, true));
        let (s, body) = get(&app, "/log/HEAD?path=src/a.rs&page=0").await;
        assert_eq!((s, body.contains("first")), (200, true));
        let (s, body) = get(&app, "/tree/HEAD/src").await;
        assert_eq!((s, body.contains("a.rs")), (200, true));
        let (s, body) = get(&app, "/blob/HEAD/src/a.rs").await;
        assert_eq!(
            (s, body.contains("<span class=\"s-keyword\">fn</span>")),
            (200, true)
        );
        assert_eq!(get(&app, "/blame/HEAD/src/a.rs").await.0, 200);
        assert_eq!(get(&app, "/refs").await.0, 200);
        let (s, body) = get(&app, "/stats").await;
        assert_eq!((s, body.contains("data-progress")), (200, true));
        assert_eq!(get(&app, "/nope/x").await.0, 404);
        assert_eq!(get(&app, "/blob/HEAD/missing").await.0, 404);
        let (_, js) = get(&app, "/api/progress").await;
        assert!(js.contains("\"ready\":false"));
        assert_eq!(get(&app, "/static/app.css").await.0, 200);

        let (s, body) = get(&app, "/log/HEAD?author=nobody%40example.com").await;
        assert_eq!((s, body.contains("author: ")), (200, true));
        let (_, body) = get(&app, "/log/HEAD?author=t%40e.x").await;
        assert!(body.contains("fix: second") && body.contains("first"));

        let (s, body) = get(&app, "/search?q=fn+main").await;
        assert_eq!((s, body.contains("<mark>fn main</mark>")), (200, true));
        let (_, body) = get(&app, "/search?q=%28&regex=1").await;
        assert!(body.contains("class=\"err\""));
        let (_, body) = get(&app, "/search?q=second&kind=commits").await;
        assert!(body.contains("fix: second"));
        let (_, body) = get(&app, "/search?q=fn&glob=*.md").await;
        assert!(body.contains("No results"));
        let (_, js) = get(&app, "/api/files/HEAD").await;
        assert_eq!(js, "[\"README.md\",\"src/a.rs\"]");
        let (_, js) = get(&app, "/api/last/HEAD/src").await;
        assert!(js.contains("\"name\":\"a.rs\"") && js.contains("fix: second"));
        let (_, js) = get(&app, "/api/refs").await;
        assert!(js.contains("\"name\":\"main\""));
        let (s, body) = get(&app, "/worktree").await;
        assert_eq!((s, body.contains("clean")), (200, true));
        assert_eq!(get(&app, "/static/nav.js").await.0, 200);
        std::fs::write(p.join("src/a.rs"), "fn main() {\n}\n// dirty\n").expect("write");
        std::fs::write(p.join("new.txt"), "x\n").expect("write");
        let (_, body) = get(&app, "/worktree").await;
        assert!(
            body.contains("src/a.rs")
                && body.contains("new.txt")
                && body.contains("class=\"diff\"")
        );
        std::fs::remove_file(p.join("new.txt")).expect("rm");
        git(p, &["checkout", "-q", "."]);

        let head = app.repo.resolve("HEAD").expect("head");
        let (s, body) = get(&app, &format!("/commit/{head}")).await;
        assert_eq!((s, body.contains("class=\"diff\"")), (200, true));

        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        for u in [
            "/stats",
            "/stats/files",
            "/stats/files?type=fix&period=1y",
            "/stats?author=t%40e.x&path=src",
            "/stats/types",
            "/stats/types?type=fix",
            "/stats/types?type=fix&scope=none&period=3m",
            "/stats/types?type=other&path=src&author=x",
            "/stats/languages",
            "/stats/frequency",
            "/stats/frequency?type=fix&path=src&period=1y",
            "/stats/contributors",
            "/stats/pulse",
            "/stats/pulse?period=1w&type=fix&bots=1&ws=1&generated=1&ignore=0&exclude=*.md",
            "/stats/hotspots",
            "/stats/hotspots?period=3m&path=src",
            "/stats/ownership",
            "/stats/patterns",
            "/compare",
            "/compare?base=HEAD~1&head=HEAD",
            "/compare?base=nope",
            "/history/src/a.rs",
            "/history/missing.rs",
            "/stats/contributors?bots=1&period=3m&author=t%40e.x",
            "/stats/languages?x=time&scale=share&from=0&to=1",
            "/",
        ] {
            let (s, body) = get(&app, u).await;
            assert_eq!(s, 200, "{u}");
            assert!(!body.contains("data-progress"), "{u}");
        }
        let (_, body) = get(&app, "/stats/types?type=fix").await;
        assert!(body.contains("class=\"chip\"") && body.contains("fix: second"));
        assert!(!body.contains("feat: first"));
        let (_, body) = get(&app, "/stats/types").await;
        assert!(body.contains("/stats/types?type=feat"));
        let (_, body) = get(&app, "/stats/contributors").await;
        assert!(body.contains("/stats/contributors?author=") && body.contains("/log/HEAD?author="));
        let (_, body) = get(&app, "/blame/HEAD/src/a.rs").await;
        assert!(body.contains("class=\"who\" href=\"/log/HEAD?author="));
    }

    #[tokio::test]
    async fn oversized_blob_links_to_raw_instead_of_rendering() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        let big = "x\n".repeat(gitterrace_git::MAX_TEXT_BYTES as usize / 2 + 10);
        std::fs::write(p.join("big.txt"), big).expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "feat: big"]);
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        let (s, body) = get(&app, "/blob/HEAD/big.txt").await;
        assert_eq!(s, 200);
        assert!(body.contains("File too large to display"));
        assert!(body.contains("/raw/HEAD/big.txt"));
        assert!(!body.contains("class=\"ch\""));
    }

    #[tokio::test]
    async fn huge_blobs_are_refused_before_they_are_loaded() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        let zeros = |name: &str, head: &[u8], mib: usize| {
            let mut f = std::fs::File::create(p.join(name)).expect("create");
            f.write_all(head).expect("head");
            f.write_all(&vec![0u8; mib << 20]).expect("fill");
        };
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend([0, 0, 0, 7, 0, 0, 0, 5, 8, 6, 0, 0, 0]);
        zeros(
            "big.png",
            &png,
            crate::routes::MAX_PREVIEW_BYTES as usize >> 20,
        );
        std::fs::write(
            p.join("big.txt"),
            "x\n".repeat(gitterrace_git::MAX_TEXT_BYTES as usize / 2 + 10),
        )
        .expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "feat: big"]);
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        assert!(crate::routes::raw_refused((256 << 20) + 1, 256));
        assert!(!crate::routes::raw_refused(256 << 20, 256));
        let (s, body) = get(&app, "/blob/HEAD/big.png").await;
        assert_eq!(s, 200);
        assert!(body.contains("class=\"media-img\"") && !body.contains("7 x 5"));
        let (s, body) = get(&app, "/blame/HEAD/big.txt").await;
        assert_eq!(s, 200);
        assert!(body.contains("too large"), "{body}");
    }

    #[tokio::test]
    async fn media_renders_inline_and_in_diffs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend([0, 0, 0, 7, 0, 0, 0, 5, 8, 6, 0, 0, 0]);
        std::fs::write(p.join("a.png"), &png).expect("write");
        std::fs::write(p.join("v.svg"), "<svg xmlns='http://www.w3.org/2000/svg'/>").expect("w");
        std::fs::write(p.join("d.pdf"), b"%PDF-1.4\n\0").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "feat: media"]);
        png.extend([1, 2, 3]);
        std::fs::write(p.join("a.png"), &png).expect("write");
        git(p, &["commit", "-qam", "fix: png"]);
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        let (_, body) = get(&app, "/blob/HEAD/a.png").await;
        assert!(
            body.contains("class=\"media-img\" src=\"/raw/HEAD/a.png\"") && body.contains("7 x 5")
        );
        let (_, body) = get(&app, "/blob/HEAD/v.svg").await;
        assert!(body.contains("media-src") && body.contains("src=\"/raw/HEAD/v.svg\""));
        let (_, body) = get(&app, "/blob/HEAD/d.pdf").await;
        assert!(body.contains("media-file") && !body.contains("<object"));
        let head = app.repo.resolve("HEAD").expect("head");
        let (_, body) = get(&app, &format!("/commit/{head}")).await;
        assert!(
            body.contains("media-diff") && body.matches("<img").count() == 2,
            "{body}"
        );
        let res = routes::router(Arc::clone(&app))
            .oneshot(
                Request::get("/raw/HEAD/d.pdf")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        let h = res.headers();
        assert_eq!(h["content-type"], "application/pdf");
        assert!(
            h["content-disposition"]
                .to_str()
                .expect("s")
                .starts_with("attachment")
        );
        let res = routes::router(Arc::clone(&app))
            .oneshot(
                Request::get("/raw/HEAD/a.png")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        let h = res.headers();
        assert_eq!(h["content-disposition"], "inline");
        assert!(
            !h["content-security-policy"]
                .to_str()
                .expect("s")
                .contains("sandbox")
        );
        let res = routes::router(Arc::clone(&app))
            .oneshot(
                Request::get("/raw/HEAD/v.svg")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        assert!(
            res.headers()["content-security-policy"]
                .to_str()
                .expect("s")
                .contains("sandbox")
        );
    }

    #[tokio::test]
    async fn app_router_nests_api_and_drops_cards_and_badges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "a\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "feat: first"]);
        let router = crate::app_router(p).expect("router");
        let status = |uri: &'static str| {
            let r = router.clone();
            async move {
                r.oneshot(
                    Request::get(uri)
                        .header("host", "localhost")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response")
                .status()
                .as_u16()
            }
        };
        assert_eq!(status("/api/v1/repo").await, 200);
        assert_eq!(status("/").await, 200);
        assert_eq!(status("/badges/commits.svg").await, 404);
        assert_eq!(status("/cards/wrapped/2025.svg").await, 404);
    }

    #[tokio::test]
    async fn one_week_selection_renders_seven_day_columns() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "x\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "feat(core): first"]);
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        for page in ["/stats", "/stats/frequency", "/stats/types", "/stats/pulse"] {
            let (s, body) = get(&app, &format!("{page}?period=1w")).await;
            assert_eq!(s, 200, "{page}");
            let chart = body.split("<svg class=\"chart\"").nth(1).expect("chart");
            let chart = chart.split("</svg>").next().unwrap_or_default();
            assert_eq!(chart.matches("class=\"col\"").count(), 7, "{page}");
            assert!(chart.contains("Mon ") || chart.contains("Sun "), "{page}");
        }
        let (_, body) = get(&app, "/stats/frequency?period=1w").await;
        assert!(body.contains("per day"), "unit comes from the axis");
    }

    #[tokio::test]
    async fn stats_files_memo_follows_head() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("first.txt"), "x\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "init"]);
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        let refresh = |app: &Arc<App>| {
            app.index
                .update(&app.repo, "HEAD", &|_| {})
                .expect("update");
            app.mark_ready();
        };
        refresh(&app);
        let (_, a) = get(&app, "/stats/files").await;
        let (_, again) = get(&app, "/stats/files").await;
        assert!(a.contains("first.txt") && a == again);

        std::fs::write(p.join("second.txt"), "y\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "more"]);
        refresh(&app);
        let (_, b) = get(&app, "/stats/files").await;
        assert!(b.contains("second.txt"), "memo outlived HEAD");
    }

    #[tokio::test]
    async fn router_rejects_foreign_and_missing_host() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "x\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "init"]);
        let router = crate::app_router(p).expect("router");
        let evil = gitterrace_testkit::on_port("evil.example");
        for (host, want) in [
            (Some(evil.as_str()), 403),
            (None, 403),
            (Some("localhost"), 200),
            (Some(gitterrace_testkit::DEFAULT_ADDR), 200),
        ] {
            let mut b = Request::get("/api/v1/repo");
            if let Some(h) = host {
                b = b.header("host", h);
            }
            let res = router
                .clone()
                .oneshot(b.body(Body::empty()).expect("request"))
                .await
                .expect("response");
            assert_eq!(res.status().as_u16(), want, "{host:?}");
        }

        let send = |uri: &str, site: Option<&str>| {
            let mut b = Request::get(uri).header("host", "localhost");
            if let Some(s) = site {
                b = b.header("sec-fetch-site", s);
            }
            router
                .clone()
                .oneshot(b.body(Body::empty()).expect("request"))
        };
        // Another port on localhost is same-site, so it is refused as well.
        for site in ["cross-site", "same-site"] {
            let res = send("/", Some(site)).await.expect("response");
            assert_eq!(res.status().as_u16(), 403, "/ {site}");
            assert_eq!(res.headers()["cross-origin-resource-policy"], "same-origin");
        }
        for site in [Some("same-origin"), Some("none"), None] {
            let res = send("/", site).await.expect("response");
            assert_eq!(res.status().as_u16(), 200, "{site:?}");
            let h = res.headers();
            assert_eq!(h["cross-origin-resource-policy"], "same-origin");
            assert_eq!(h["x-content-type-options"], "nosniff");
            let csp = h["content-security-policy"].to_str().expect("ascii");
            assert!(csp.contains("frame-ancestors 'none'") && csp.contains("script-src 'self'"));
        }
        let res = send("/api/v1/repo", None).await.expect("response");
        assert_eq!(res.headers()["x-content-type-options"], "nosniff");
        assert!(!res.headers().contains_key("content-security-policy"));
    }

    fn fixture(name: &str, root: &std::path::Path) -> std::path::PathBuf {
        Fixture::standard(&root.join(name)).path().to_owned()
    }

    async fn fetch(router: &axum::Router, uri: &str, host: &str) -> (u16, String) {
        let res = router
            .clone()
            .oneshot(
                Request::get(uri)
                    .header("host", host)
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        let s = res.status().as_u16();
        let b = res.into_body().collect().await.expect("body").to_bytes();
        (s, String::from_utf8_lossy(&b).into_owned())
    }

    /// Same-origin `href`, `src`, `action` and `data-last` values in a page.
    fn links(html: &str) -> Vec<String> {
        let mut out = Vec::new();
        for attr in ["href=\"", "src=\"", "action=\"", "data-last=\""] {
            for (i, _) in html.match_indices(attr) {
                let rest = &html[i + attr.len()..];
                let v = &rest[..rest.find('"').unwrap_or(0)];
                if v.starts_with('/') && !v.starts_with("//") {
                    out.push(v.replace("&amp;", "&").replace("&#38;", "&"));
                }
            }
        }
        out
    }

    /// Crawls every page of both repositories and fails on any link leaving its `/r/NAME`.
    #[tokio::test]
    async fn multi_repo_links_stay_inside_their_repo() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = fixture("alpha", dir.path());
        let b = fixture("beta", dir.path());
        let router = crate::multi_router(vec![a, b]).expect("router");
        let (s, idx) = fetch(&router, "/", "localhost").await;
        assert_eq!(s, 200);
        assert!(idx.contains("/r/alpha/") && idx.contains("/r/beta/"));
        for name in ["alpha", "beta"] {
            let prefix = format!("/r/{name}/");
            let mut todo = vec![prefix.clone()];
            let mut seen = std::collections::HashSet::new();
            let mut escapes = Vec::new();
            while let Some(u) = todo.pop() {
                if !seen.insert(u.clone()) || seen.len() > 400 {
                    continue;
                }
                let (st, body) = fetch(&router, &u, "localhost").await;
                assert!(st < 500, "{u}: {st} {body}");
                if !body.contains("<html") {
                    continue;
                }
                for l in links(&body) {
                    if l == "/" {
                        continue; // the brand links to the repository index on purpose
                    }
                    if !l.starts_with(&prefix) {
                        escapes.push(format!("{u} -> {l}"));
                    } else if !l.contains("/static/") && !l.contains("/raw/") {
                        todo.push(l.split('#').next().unwrap_or("").to_owned());
                    }
                }
            }
            assert!(seen.len() > 10, "crawl too shallow: {}", seen.len());
            assert!(
                escapes.is_empty(),
                "links escape {prefix}:\n{}",
                escapes.join("\n")
            );
        }
        let (s, js) = fetch(&router, "/r/beta/api/files/HEAD", "localhost").await;
        assert_eq!((s, js.contains("src/a.rs")), (200, true));
        assert_eq!(
            fetch(&router, "/r/alpha/api/v1/repo", "localhost").await.0,
            200
        );
        assert_eq!(
            fetch(&router, "/r/alpha/raw/HEAD/src/a.rs", "localhost")
                .await
                .0,
            200
        );
        assert_eq!(fetch(&router, "/static/app.css", "localhost").await.0, 200);
    }

    #[tokio::test]
    async fn multi_repo_names_resolve_only_through_the_table() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = fixture("alpha", dir.path());
        let b = fixture("beta", dir.path());
        let router = crate::multi_router(vec![a, b]).expect("router");
        for u in [
            "/r/../alpha/",
            "/r/..%2Falpha/",
            "/r/%2e%2e/alpha/",
            "/r/%2E%2E/",
            "/r/alpha%2F..%2Fbeta/",
            "/r/./",
            "/r/gamma/",
            "/r/ALPHA/",
            "/r/alpha%00/",
            "/r//",
            "/r/alpha/../../etc/passwd",
            "/r/alpha/raw/HEAD/../../../etc/passwd",
        ] {
            let (s, body) = fetch(&router, u, "localhost").await;
            assert!(matches!(s, 400 | 404), "{u}: {s} {body}");
            assert!(!body.contains("root:"), "{u}");
        }
        assert_eq!(fetch(&router, "/r/alpha", "localhost").await.0, 308);
    }

    #[tokio::test]
    async fn multi_repo_host_guard_covers_nested_routes() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = fixture("alpha", dir.path());
        let b = fixture("beta", dir.path());
        let router = crate::multi_router(vec![a, b]).expect("router");
        for u in [
            "/",
            "/api/repos",
            "/static/app.css",
            "/r/alpha/",
            "/r/alpha/api/v1/repo",
            "/r/alpha/raw/HEAD/src/a.rs",
            "/r/beta/api/files/HEAD",
            "/r/nope/",
        ] {
            assert_eq!(
                fetch(&router, u, &gitterrace_testkit::on_port("evil.example"))
                    .await
                    .0,
                403,
                "{u}"
            );
        }
        assert_eq!(
            fetch(
                &router,
                "/r/alpha/api/v1/repo",
                gitterrace_testkit::DEFAULT_ADDR
            )
            .await
            .0,
            200
        );
    }

    #[test]
    fn duplicate_folder_names_get_a_suffix() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = fixture("same", &dir.path().join("x"));
        let b = fixture("same", &dir.path().join("y"));
        let t = crate::multi::Table::new(
            vec![a, b],
            crate::app::Env::new(gitterrace_config::Config::default()),
        )
        .expect("table");
        assert!(t.get("same").is_some() && t.get("same-2").is_some());
    }

    #[tokio::test]
    async fn single_repo_urls_are_unprefixed() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = fixture("solo", dir.path());
        let router = crate::multi_router(vec![a]).expect("router");
        let (_, body) = fetch(&router, "/", "localhost").await;
        assert!(body.contains("href=\"/log/HEAD\"") && !body.contains("/r/solo"));
        assert!(body.contains("<meta name=\"gt-base\" content=\"\">"));
        assert!(!body.contains("repobtn"));
    }

    fn rev_fixture(root: &std::path::Path) -> std::path::PathBuf {
        let f = Fixture::standard(&root.join("solo"))
            .branch("side")
            .write("side.txt", "s\n")
            .commit("feat: only on side")
            .checkout("main")
            .tag("v1", "Release one\n\nnotes here")
            .worktree(&root.join("wt"), "side")
            .write("README.md", "dirty\n")
            .stash("wip stash");
        f.path().to_owned()
    }

    fn ready_app(p: &std::path::Path) -> Arc<App> {
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        app
    }

    /// A cache a newer build wrote is explained on the insights pages, served as 200, and left
    /// byte-identical: an older server running next to a newer one must not clobber it.
    #[tokio::test]
    async fn a_newer_cache_is_explained_not_clobbered() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = rev_fixture(dir.path());
        let built = ready_app(&p);
        let cache = built.index.dir().to_owned();
        drop(built);
        std::fs::write(cache.join("VERSION"), "99\n").expect("stamp");
        let snapshot = |d: &std::path::Path| {
            let mut v: Vec<_> = std::fs::read_dir(d)
                .expect("ls")
                .map(|e| {
                    let e = e.expect("entry");
                    (e.file_name(), std::fs::read(e.path()).expect("read"))
                })
                .collect();
            v.sort();
            v
        };
        let before = snapshot(&cache);
        let repo = gitterrace_git::Repo::discover(&p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        // The server marks the app ready once its update attempt ends, failed or not.
        app.mark_ready();
        for uri in ["/stats", "/stats/types", "/stats/hotspots"] {
            let (s, body) = get(&app, uri).await;
            assert_eq!(s, 200, "{uri}");
            assert!(body.contains("format 99"), "{uri}: {body}");
        }
        assert!(snapshot(&cache) == before, "cache files changed");
    }

    #[tokio::test]
    async fn git_state_pages_and_banner() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = rev_fixture(dir.path());
        let app = ready_app(&p);
        let (s, body) = get(&app, "/refs").await;
        assert_eq!(s, 200);
        assert!(
            body.contains(">side</a>") && body.contains("default"),
            "{body}"
        );
        assert!(
            body.contains("/compare?base=main&#38;head=side"),
            "ahead links to compare"
        );
        assert!(body.contains("Release one") && body.contains("/tag/v1"));
        let (s, body) = get(&app, "/tag/v1").await;
        assert!(s == 200 && body.contains("notes here") && body.contains("Tagged by"));
        assert_eq!(get(&app, "/tag/nope").await.0, 404);
        let (s, body) = get(&app, "/worktrees").await;
        assert!(
            s == 200 && body.contains("/wt</code>") && body.contains("/log/side"),
            "{body}"
        );
        let (s, body) = get(&app, "/stashes").await;
        assert!(s == 200 && body.contains("stash@{0}") && body.contains("wip stash"));
        assert!(
            body.contains("id=\"headstate\" hidden>"),
            "the banner is hidden on a branch"
        );
        let head = app.repo.resolve("HEAD").expect("head");
        git(&p, &["checkout", "-q", "--detach"]);
        let (_, body) = get(&app, "/").await;
        let short = gitterrace_git::short_id(&head);
        assert!(
            body.contains(&format!(
                "id=\"headstate\"><div class=\"wrap\">HEAD is detached at <a class=\"sha\" href=\"/commit/{head}\" data-tip=\"{head}\">{short}</a>"
            )),
            "{body}"
        );
    }

    /// The selected ref survives every link to a page computed from it, and those pages read
    /// the selected ref, not HEAD.
    #[tokio::test]
    async fn rev_is_preserved_by_every_link() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = rev_fixture(dir.path());
        let app = ready_app(&p);
        let (s, body) = get(&app, "/stats?rev=side").await;
        assert!(
            s == 200 && body.contains("data-progress"),
            "side is not indexed yet"
        );
        app.index
            .update(&app.repo, "side", &|_| {})
            .expect("update");
        let (_, main) = get(&app, "/stats/types").await;
        let (_, side) = get(&app, "/stats/types?rev=side").await;
        assert!(side.contains("only on side") && !main.contains("only on side"));
        let (_, side) = get(&app, "/?rev=side").await;
        assert!(
            side.contains("side.txt"),
            "overview lists the selected ref's files"
        );
        let mut todo = vec!["/?rev=side".to_owned(), "/stats?rev=side".to_owned()];
        let mut seen = std::collections::HashSet::new();
        let mut lost = Vec::new();
        while let Some(u) = todo.pop() {
            if !seen.insert(u.clone()) || seen.len() > 300 {
                continue;
            }
            let (st, body) = get(&app, &u).await;
            assert!(st < 500, "{u}: {st}");
            if !body.contains("<html") {
                continue;
            }
            // The brand goes to the repository index on purpose, whatever ref is selected.
            let body = body.replacen("class=\"brand\" href=\"/\"", "class=\"brand\"", 1);
            for l in links(&body) {
                let path = l.split(['?', '#']).next().unwrap_or_default().to_owned();
                let follows = path == "/"
                    || path == "/stats"
                    || path.starts_with("/stats/")
                    || path.starts_with("/history/");
                if follows && !l.contains("rev=side") {
                    lost.push(format!("{u} -> {l}"));
                } else if follows {
                    todo.push(l.split('#').next().unwrap_or_default().to_owned());
                }
            }
        }
        assert!(seen.len() > 10, "crawl too shallow: {}", seen.len());
        assert!(lost.is_empty(), "links drop ?rev=:\n{}", lost.join("\n"));
    }

    /// Methods a route may answer besides GET and HEAD, as (path, method). gitterrace is
    /// read-only, so a write route is an owner decision recorded here, never an accident.
    /// The one entry is the action runner, mounted only when the user configured actions.
    #[cfg(unix)]
    const WRITE_ALLOWLIST: [(&str, &str); 1] = [(crate::actions::RUN_PATH, "POST")];

    /// Every page a crawl reaches, the API and the assets refuse any method but GET and HEAD,
    /// with actions off and on. With them off the allowlisted route does not exist at all.
    #[cfg(unix)]
    #[tokio::test]
    async fn every_route_is_read_only() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = fixture("alpha", dir.path());
        let b = fixture("beta", dir.path());
        let cfg = actions_tests::config_dir(actions_tests::ECHO);
        let off = crate::multi_router(vec![a.clone(), b.clone()]).expect("router");
        let on = crate::build_multi(
            vec![a, b],
            None,
            Some(&cfg.path().join("actions.toml")),
            &crate::app::Env::new(gitterrace_config::Config::default()),
        )
        .expect("router")
        .0;
        for (router, actions) in [(off.clone(), false), (on, true)] {
            crawl_read_only(&router, actions).await;
        }
        let res = off
            .oneshot(
                Request::post(crate::actions::RUN_PATH)
                    .header("host", "localhost")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        assert_eq!(res.status(), 404, "run route exists with actions off");
    }

    #[cfg(unix)]
    async fn crawl_read_only(router: &axum::Router, actions: bool) {
        let router = router.clone();
        let mut todo = vec![
            "/".to_owned(),
            "/r/alpha/".to_owned(),
            "/api/repos".to_owned(),
        ];
        todo.extend(
            [
                "/api/v1/repo",
                "/api/v1/refs",
                "/api/progress",
                "/api/refs",
                "/api/files/HEAD",
            ]
            .map(|p| format!("/r/alpha{p}")),
        );
        let mut seen = std::collections::HashSet::new();
        let mut writable = Vec::new();
        // The run route renders no link, so the crawl is seeded with it; GET on it is 405.
        todo.push(crate::actions::RUN_PATH.to_owned());
        while let Some(u) = todo.pop() {
            if !seen.insert(u.clone()) || seen.len() > 300 {
                continue;
            }
            let (get_status, body) = fetch(&router, &u, "localhost").await;
            for l in links(&body) {
                todo.push(l.split('#').next().unwrap_or_default().to_owned());
            }
            for m in ["POST", "PUT", "PATCH", "DELETE"] {
                let res = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(m)
                            .uri(&u)
                            .header("host", "localhost")
                            .body(Body::empty())
                            .expect("req"),
                    )
                    .await
                    .expect("res");
                let path = u.split('?').next().unwrap_or_default();
                let allowed =
                    actions && WRITE_ALLOWLIST.iter().any(|(p, am)| *p == path && *am == m);
                // 405 is a route that exists for GET only. A URL that GET cannot find either
                // (a `data-last` prefix) may answer 404; any other status means a handler took
                // the write.
                let code = res.status().as_u16();
                let refused = code == 405 || (code == 404 && get_status == 404);
                if !refused && !allowed {
                    writable.push(format!("{m} {u}: {}", res.status()));
                }
            }
        }
        assert!(seen.len() > 20, "crawl too shallow: {}", seen.len());
        assert!(
            writable.is_empty(),
            "routes accept writes:\n{}",
            writable.join("\n")
        );
    }
}
