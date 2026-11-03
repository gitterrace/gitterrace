//! HTTP server: server-rendered pages over `gitterrace-git` and `gitterrace-index`.

mod about;
mod actions;
mod activity;
mod add;
mod app;
mod bounded;
mod caches;
mod charts;
mod commit_types;
mod compare;
mod error;
mod far;
mod filter;
mod fmt;
mod gitstate;
mod guard;
mod highlight;
mod history;
mod hotspots;
mod insights;
mod languages;
mod live;
mod markdown;
mod media;
mod multi;
mod overview_side;
mod ownership;
mod patterns;
mod public;
mod registry;
mod reload;
mod routes;
mod search;
pub mod sitemap;
mod views;
#[cfg(test)]
mod wiring_tests;

use std::{
    io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

pub use add::TOKEN_HEADER as ADD_TOKEN_HEADER;
pub use fmt::esc as escape_html;
use gitterrace_config::Config;
use gitterrace_git::Repo;
use gitterrace_index::Index;
pub use guard::CSP as PAGE_CSP;
pub use highlight::languages as highlighted_languages;
pub use media::plain_media;
pub use multi::discover_repos;
pub use registry::{
    Entry as RegistryEntry, add_token, candidates as registry_candidates, state_dir,
};
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

/// Router for one or more repositories, each under `/r/NAME`; `/` forwards to a lone one and
/// indexes several. Same Host rules. Names come from folder names, not the registry.
pub fn multi_router(paths: Vec<PathBuf>) -> io::Result<axum::Router> {
    Ok(build_multi(paths, None, None, &Env::new(Config::default()))?.0)
}

/// One repository's pages and `/api/v1`, with links under `base`. Starts its
/// index refresher, so call it inside a tokio runtime.
#[cfg(test)]
fn repo_router(
    repo_path: &Path,
    base: String,
    key: String,
    live: Option<Arc<actions::Live>>,
    env: &Env,
) -> io::Result<axum::Router> {
    Ok(repo_router_tasks(repo_path, base, key, live, env)?.0)
}

/// [`repo_router`] and the handles of the background tasks it started (index refresher, idle
/// release, warm-up), so a repository taken off a running server stops them and its memory
/// returns.
fn repo_router_tasks(
    repo_path: &Path,
    base: String,
    key: String,
    live: Option<Arc<actions::Live>>,
    env: &Env,
) -> io::Result<(axum::Router, Vec<tokio::task::AbortHandle>)> {
    let cfg = env.cfg();
    let (text_bytes, blame_bytes) = app::repo_limits(&cfg);
    let repo = Repo::discover(repo_path)
        .map_err(io::Error::other)?
        .with_limits(text_bytes, blame_bytes)
        .with_workers(cfg.index.threads);
    if env.public() {
        repo.restrict_to_committed();
    }
    let index = Index::open(&repo)?.with_threads(cfg.index.threads);
    let mut app = App::configured(repo, index, env);
    app.base = base;
    app.key = key;
    app.actions = live;
    let app = Arc::new(app);
    env.register(&app);
    let mut tasks = vec![
        tokio::spawn(app::keep_index_fresh(Arc::clone(&app))).abort_handle(),
        tokio::spawn(app::release_when_idle(
            Arc::downgrade(&app),
            app::RECORDS_IDLE,
        ))
        .abort_handle(),
    ];
    let api = gitterrace_api::ApiState::open(repo_path)
        .map_err(io::Error::other)?
        .configured(&env.cfg());
    let api = if env.public() {
        api.committed_only()
    } else {
        api
    };
    // A reload reaches the API through the app: both count with the same `insights.*`.
    app.set_api(api.clone());
    let pages = routes::router(Arc::clone(&app)).nest("/api/v1", gitterrace_api::router(api));
    let pages = if env.public() {
        pages.route_layer(axum::middleware::from_fn(public::only_public))
    } else {
        pages
    };
    tasks.push(tokio::spawn(prewarm(Arc::clone(&app), pages.clone())).abort_handle());
    Ok((pages, tasks))
}

/// A failed bind with the address and, when the port is taken, what to do about it. The kind
/// is kept, so a caller can still tell the cases apart.
fn bind_error(addr: SocketAddr, e: &io::Error) -> io::Error {
    let hint = if e.kind() == io::ErrorKind::AddrInUse {
        "; another git terrace or other server is using that port: stop it, or pass --addr with another port (127.0.0.1:0 picks a free one)"
    } else {
        ""
    };
    io::Error::new(e.kind(), format!("cannot listen on {addr}: {e}{hint}"))
}

/// Unfiltered pages rendered in the background whenever HEAD moves.
///
/// Each costs 40-90 ms to
/// build on a 14k-commit repo, then a few ms from the memo, so no visitor pays the build.
/// `export` seeds its crawl from the same list, so a static copy has every insights page.
pub const WARM_PAGES: [&str; 9] = [
    "/",
    "/stats",
    "/stats/types",
    "/stats/languages",
    "/stats/frequency",
    "/stats/contributors",
    "/stats/hotspots",
    "/stats/ownership",
    "/stats/patterns",
];

/// Opens the insights source for HEAD each time the index moves, then renders the unfiltered
/// pages once, so the first visit to any of them is served from the memo. Sleeps between moves.
async fn prewarm(app: Arc<App>, pages: axum::Router) {
    use tower::ServiceExt;
    let by = Arc::clone(&app);
    app::on_refreshed(app, move || {
        let (app, pages) = (Arc::clone(&by), pages.clone());
        async move {
            // The warm-up is no visitor: what it opens goes one idle period after it started.
            let _warm = app.warming();
            let ready = app::blocking(&app, |a| {
                let head = a.repo.resolve("HEAD")?;
                Ok(a.source(&head)?.is_some())
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
            routes::prewarm_blame(app).await;
        }
    })
    .await;
}

fn guarded(router: axum::Router, gate: guard::Gate) -> axum::Router {
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
        .layer(axum::middleware::from_fn_with_state(
            Arc::new(gate),
            guard::guard,
        ))
}

fn build(
    repo_path: &Path,
    extra: Option<IpAddr>,
    actions_file: Option<&Path>,
    env: &Env,
) -> io::Result<(axum::Router, String)> {
    build_server(
        vec![repo_path.to_path_buf()],
        extra,
        actions_file,
        env,
        None,
        None,
    )
}

fn build_multi(
    paths: Vec<PathBuf>,
    extra: Option<IpAddr>,
    actions_file: Option<&Path>,
    env: &Env,
) -> io::Result<(axum::Router, String)> {
    build_server(paths, extra, actions_file, env, None, None)
}

/// The whole server: a repository table, every repository under `/r/NAME`, so `add` (the secret
/// `POST /_repos` checks, `None` leaves the route unmounted) grows it without moving a link.
/// `state` is where names and additions are recorded.
fn build_server(
    paths: Vec<PathBuf>,
    extra: Option<IpAddr>,
    actions_file: Option<&Path>,
    env: &Env,
    add: Option<gitterrace_actions::Token>,
    state: Option<PathBuf>,
) -> io::Result<(axum::Router, String)> {
    let n = paths.len();
    let mut table = multi::Table::new(paths, env.clone(), state.clone())?;
    let targets = table.targets()?;
    let roots: Vec<PathBuf> = targets.iter().map(|t| t.repo.root()).collect();
    // A public server never reads actions.toml: no run route, no Send to targets.
    let actions_file = actions_file.filter(|_| !env.public());
    if env.public() {
        for t in &targets {
            t.repo.restrict_to_committed();
        }
    }
    // Behind a proxy a public server sits on loopback and every request is remote.
    let local = extra.is_none() && !env.public();
    let (live, status) = actions::start(actions_file, &roots, local);
    env.set_actions_status(&status);
    table.live.clone_from(&live);
    let sends = actions::sends(live.as_ref(), actions_file, &roots);
    env.set_sends(&sends);
    let site = Arc::new(actions::Site {
        targets: std::sync::RwLock::new(targets.into_iter().map(Arc::new).collect()),
        live: live.clone(),
        status,
        sends,
        env: env.clone(),
    });
    let table = Arc::new(table);
    table.start_all();
    let router = multi::router(Arc::clone(&table));
    let shown = fmt::count(n, "repository", "repositories");
    let mut router = router.merge(actions::router(Arc::clone(&site)));
    // A non-loopback bind or a public server never takes additions: the token is the only
    // gate a remote client would face.
    if let Some(token) = add.filter(|_| local) {
        router = router.merge(add::router(Arc::new(add::Adder {
            table,
            site,
            token,
            actions_file: actions_file.map(Path::to_path_buf),
            state,
        })));
    }
    let router = if env.public() {
        router.route_layer(axum::middleware::from_fn(public::only_public))
    } else {
        router
    };
    let gate = guard::Gate::new(extra, &env.cfg());
    Ok((guarded(router, gate), shown))
}

/// Records the served repositories for `git terrace open`. Best effort: a read-only or odd
/// state folder must not stop the server, so a failure is only logged.
fn remember(dir: Option<&Path>, paths: &[PathBuf]) {
    let Some(dir) = dir else {
        return;
    };
    let repos: Vec<Repo> = paths
        .iter()
        .filter_map(|p| Repo::discover(p).ok())
        .collect();
    let refs: Vec<&Repo> = repos.iter().collect();
    if let Err(e) = registry::record(dir, &refs) {
        tracing::warn!(
            "not recording served repositories in {}: {e}",
            dir.display()
        );
    }
}

/// Starts serving until ctrl-c.
///
/// `actions` is the user's `actions.toml`; `None` (no config
/// folder, or `--no-actions`) leaves the run route unmounted, as does a non-loopback bind.
/// `watched` is the `config.toml` to reload while running, see [`Watched`].
pub async fn serve(
    repo_paths: Vec<PathBuf>,
    addr: SocketAddr,
    actions: Option<PathBuf>,
    cfg: Config,
    watched: Option<Watched>,
) -> io::Result<()> {
    serve_in(
        repo_paths,
        addr,
        actions,
        cfg,
        watched,
        registry::state_dir(),
    )
    .await
}

/// [`serve`] with the state folder (registry and add secret) named, `None` for none.
pub async fn serve_in(
    repo_paths: Vec<PathBuf>,
    addr: SocketAddr,
    actions: Option<PathBuf>,
    cfg: Config,
    watched: Option<Watched>,
    state: Option<PathBuf>,
) -> io::Result<()> {
    let extra = (!addr.ip().is_loopback()).then(|| addr.ip());
    // Also checked by `Config::validate`; repeated here because `serve` takes any `Config`.
    if extra.is_some() && !cfg.server.public {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{addr} is reachable from other machines. Pass --public (server.public = true) to serve read-only there"
            ),
        ));
    }
    let public = cfg.server.public;
    tokio::task::spawn_blocking({
        let (paths, state) = (repo_paths.clone(), state.clone());
        move || remember(state.as_deref(), &paths)
    });
    let env = Env::new(cfg);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| bind_error(addr, &e))?;
    let addr = listener.local_addr()?;
    // The secret `open` reads to add a repository. Without a state folder, or when it cannot
    // be written, the route stays unmounted and `open` starts a server of its own.
    let add = match (&state, extra) {
        (Some(dir), None) if !public => publish_add_token(dir, addr.port()),
        _ => None,
    };
    let published = add.is_some();
    let (router, name) = build_server(
        repo_paths,
        extra,
        actions.as_deref(),
        &env,
        add,
        state.clone(),
    )?;
    if let Some(w) = watched {
        tokio::spawn(reload::run(reload::Reloader::new(w), env.clone()));
    }
    if public {
        tracing::warn!(
            "public: anyone who can reach this address reads the committed history. Working tree, stashes, live updates and actions are off"
        );
    }
    tracing::info!("serving {name} on http://{addr}");
    let served = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
            // Open tabs hold their streams; closing them lets the server stop.
            env.shut_down();
        })
        .await;
    if let (true, Some(dir)) = (published, &state) {
        registry::remove_add_token(dir, addr.port());
    }
    served
}

/// A fresh secret for `POST /_repos`, written where only this user can read it.
fn publish_add_token(dir: &Path, port: u16) -> Option<gitterrace_actions::Token> {
    let token = gitterrace_actions::Token::generate()
        .map_err(|e| tracing::warn!("no add token: {e}"))
        .ok()?;
    match registry::write_add_token(dir, port, token.as_str()) {
        Ok(()) => Some(token),
        Err(e) => {
            tracing::warn!("adding repositories is off: {}: {e}", dir.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::{Fixture, git};
    use std::{path::PathBuf, sync::Arc};

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[cfg(unix)]
    use crate::actions::actions_tests;
    use crate::{app::App, routes};
    use gitterrace_git::Repo;

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
        // A tab reloads the page; every one lands on the card it switches, not the top.
        assert!(body.contains("id=\"about\""));
        assert!(body.contains("?doc=license#about\""));
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
    #[expect(
        clippy::too_many_lines,
        reason = "one linear sequence, splitting it hides the flow"
    )]
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
            "/stats?type=fix&period=1y",
            "/stats?author=t%40e.x&path=src",
            "/stats/types",
            "/stats/types?type=fix",
            "/stats/types?type=fix&scope=none&period=3m",
            "/stats/types?type=other&path=src&author=x",
            "/stats/languages",
            "/stats/frequency",
            "/stats/frequency?type=fix&path=src&period=1y",
            "/stats/contributors",
            "/stats?period=1w&type=fix&bots=1&ws=1&generated=1&ignore=0&exclude=*.md",
            "/stats/types?bots=1&ws=1&page=1",
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
        // Every page that names an author links to their log, branch rows included.
        let (_, body) = get(&app, "/refs").await;
        assert!(
            body.contains("class=\"who\" href=\"/log/HEAD?author="),
            "refs"
        );
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
        let base = gitterrace_testkit::base(&router).await;
        let status = |uri: &'static str| {
            let (r, uri) = (router.clone(), format!("{base}{uri}"));
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
        assert_eq!(status("/r/x/").await, 404, "no page under a page");
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
        for page in ["/stats", "/stats/frequency", "/stats/types"] {
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
    async fn hotspots_memo_follows_head() {
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
        let (_, a) = get(&app, "/stats/hotspots").await;
        let (_, again) = get(&app, "/stats/hotspots").await;
        assert!(a.contains("first.txt") && a == again);

        std::fs::write(p.join("second.txt"), "y\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "more"]);
        refresh(&app);
        let (_, b) = get(&app, "/stats/hotspots").await;
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
        let base = gitterrace_testkit::base(&router).await;
        let evil = gitterrace_testkit::on_port("evil.example");
        for (host, want) in [
            (Some(evil.as_str()), 403),
            (None, 403),
            (Some("localhost"), 200),
            (Some(gitterrace_testkit::DEFAULT_ADDR), 200),
        ] {
            let mut b = Request::get(format!("{base}/api/v1/repo"));
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
            let res = send(&format!("{base}/"), site).await.expect("response");
            assert_eq!(res.status().as_u16(), 200, "{site:?}");
            let h = res.headers();
            assert_eq!(h["cross-origin-resource-policy"], "same-origin");
            assert_eq!(h["x-content-type-options"], "nosniff");
            let csp = h["content-security-policy"].to_str().expect("ascii");
            assert!(csp.contains("frame-ancestors 'none'") && csp.contains("script-src 'self'"));
        }
        let res = send(&format!("{base}/api/v1/repo"), None)
            .await
            .expect("response");
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
                    // The brand links to the repository index, and stylesheets and scripts
                    // are the server's, shared by every repository: both at the root on purpose.
                    if l == "/" || l.starts_with("/static/") {
                        continue;
                    }
                    if !l.starts_with(&prefix) {
                        escapes.push(format!("{u} -> {l}"));
                    } else if !l.contains("/raw/") {
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

    /// Guard (c): two repositories with one folder name. The first to arrive keeps the plain
    /// name, the newcomer gets `-<hash6>`, and both names survive a restart in the other order:
    /// they come from the registry, never from arrival order.
    #[test]
    fn a_clashing_folder_name_is_first_come_and_frozen() {
        let dir = tempfile::tempdir().expect("tmp");
        let state = dir.path().join("state");
        let z = fixture("app", &dir.path().join("z"));
        let a = fixture("app", &dir.path().join("a"));
        let env = || crate::app::Env::new(gitterrace_config::Config::default());
        let names = |paths: Vec<PathBuf>| {
            let t =
                crate::multi::Table::new(paths.clone(), env(), Some(state.clone())).expect("table");
            paths
                .iter()
                .map(|p| {
                    let root = Repo::discover(p).expect("repo").root();
                    t.name_of(&root).expect("served")
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(names(vec![z.clone()]), ["app"], "alone it is plain");
        let both = names(vec![z.clone(), a.clone()]);
        assert_eq!(both[0], "app", "the first come keeps it");
        assert!(
            both[1].starts_with("app-") && both[1].len() == 10,
            "{both:?}"
        );
        let reversed = names(vec![a, z]);
        assert_eq!(reversed, [both[1].clone(), both[0].clone()], "frozen");
    }

    /// Without a registry the same rule runs against the table: a clash still gets a stable
    /// path-derived suffix, never a counter that depends on order.
    #[test]
    fn without_a_registry_a_clash_is_named_by_its_path() {
        let dir = tempfile::tempdir().expect("tmp");
        let x = fixture("same", &dir.path().join("x"));
        let y = fixture("same", &dir.path().join("y"));
        let env = || crate::app::Env::new(gitterrace_config::Config::default());
        let t = crate::multi::Table::new(vec![x.clone(), y.clone()], env(), None).expect("t");
        let y_root = Repo::discover(&y).expect("repo").root();
        let y_name = t.name_of(&y_root).expect("served");
        assert!(t.get("same").is_some() && y_name.starts_with("same-"));
        let t = crate::multi::Table::new(vec![y, x], env(), None).expect("t");
        assert_eq!(
            t.name_of(&y_root).as_deref(),
            Some("same"),
            "no registry, no memory"
        );
    }

    #[tokio::test]
    async fn a_single_repository_lives_under_its_name() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = fixture("solo", dir.path());
        let router = crate::multi_router(vec![a]).expect("router");
        let (s, _) = fetch(&router, "/", "localhost").await;
        assert_eq!(s, 302, "/ forwards to the one repository");
        let (_, body) = fetch(&router, "/r/solo/", "localhost").await;
        assert!(body.contains("href=\"/r/solo/log/HEAD\""));
        assert!(body.contains("<meta name=\"gt-base\" content=\"/r/solo\">"));
        assert!(
            body.contains("id=\"reposwitch\" hidden"),
            "one repository, no switcher"
        );
        assert_eq!(
            fetch(&router, "/log/HEAD", "localhost").await.0,
            404,
            "no unprefixed pages"
        );
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
        // A ref a few commits off HEAD is served from the segments at once, no indexing page.
        let (s, body) = get(&app, "/stats?rev=side").await;
        assert!(
            s == 200 && !body.contains("data-progress"),
            "side shows the indexing page: {body}"
        );
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

    fn public_cfg() -> gitterrace_config::Config {
        let mut c = gitterrace_config::Config::default();
        c.server.public = true;
        c.server.hosts = vec!["git.example.com".to_owned()];
        c
    }

    /// A public server, crawled from `/` through every link it renders and seeded with every
    /// local route: each answer is a classified public route, none names the host's folders or
    /// local state, local routes are 404, writes are 405 before routing.
    #[cfg(unix)]
    #[tokio::test]
    async fn public_server_shows_committed_history_only() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = fixture("alpha", dir.path());
        std::fs::write(p.join("wip.txt"), "uncommitted-secret\n").expect("write");
        git(&p, &["stash", "-q", "-u"]);
        std::fs::write(p.join("dirty.txt"), "dirty-secret\n").expect("write");
        let cfg = actions_tests::config_dir(actions_tests::ECHO);
        let env = crate::app::Env::new(public_cfg());
        let router = crate::build(&p, None, Some(&cfg.path().join("actions.toml")), &env)
            .expect("router")
            .0;
        let root = std::fs::canonicalize(&p).expect("canon");
        let leaks = [
            root.to_string_lossy().into_owned(),
            dir.path().to_string_lossy().into_owned(),
            "uncommitted-secret".to_owned(),
            "dirty-secret".to_owned(),
        ];
        let mut todo = vec![
            "/".to_owned(),
            "/r/alpha/".to_owned(),
            "/r/alpha/api/v1/repo".to_owned(),
            "/r/alpha/api/v1/refs".to_owned(),
        ];
        let mut seen = std::collections::HashSet::new();
        let mut bad = Vec::new();
        while let Some(u) = todo.pop() {
            if !seen.insert(u.clone()) || seen.len() > 300 {
                continue;
            }
            let (s, body) = fetch(&router, &u, "git.example.com").await;
            if s == 200 {
                for l in links(&body) {
                    todo.push(l.split('#').next().unwrap_or_default().to_owned());
                }
            }
            if let Some(l) = leaks.iter().find(|l| body.contains(l.as_str())) {
                bad.push(format!("{u} shows {l}"));
            }
            for l in ["/worktree", "/stashes", "/events", "/_send", "/_actions"] {
                if body.contains(&format!("href=\"{l}"))
                    || body.contains(&format!("href=\"/r/alpha{l}"))
                {
                    bad.push(format!("{u} links {l}"));
                }
            }
        }
        assert!(seen.len() > 20, "crawl too shallow: {}", seen.len());
        for u in [
            "/r/alpha/worktree",
            "/r/alpha/worktrees",
            "/r/alpha/stashes",
            "/r/alpha/events",
            "/_actions/status",
            "/r/alpha/_send/targets",
            "/r/alpha/commit/stash",
            "/r/alpha/log/refs/stash",
            "/r/alpha/tree/stash",
            "/r/alpha/api/v1/log?rev=stash",
            "/r/alpha/api/v1/blob?rev=stash&path=wip.txt",
        ] {
            let (s, body) = fetch(&router, u, "git.example.com").await;
            if s == 200 || body.contains("uncommitted-secret") {
                bad.push(format!("{u}: {s}"));
            }
        }
        for (m, u) in [
            ("POST", crate::actions::RUN_PATH),
            ("POST", "/_repos"),
            ("PUT", "/"),
        ] {
            let res = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(m)
                        .uri(u)
                        .header("host", "git.example.com")
                        .body(Body::empty())
                        .expect("req"),
                )
                .await
                .expect("res");
            if res.status() != 405 {
                bad.push(format!("{m} {u}: {}", res.status()));
            }
        }
        assert!(bad.is_empty(), "public server leaks:\n{}", bad.join("\n"));
        // The proxy name works, a cross-site link into it works, another name does not.
        let res = router
            .clone()
            .oneshot(
                Request::get("/r/alpha/")
                    .header("host", "GIT.example.com:443")
                    .header("sec-fetch-site", "cross-site")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        assert_eq!(res.status(), 200);
        assert_eq!(fetch(&router, "/", "evil.example.com").await.0, 403);
    }

    /// A taken port names the address and the way out, not a bare os error.
    #[tokio::test]
    async fn a_busy_port_names_the_address_and_a_hint() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = fixture("alpha", dir.path());
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = taken.local_addr().expect("addr");
        let e = crate::serve_in(
            vec![p],
            addr,
            None,
            gitterrace_config::Config::default(),
            None,
            None,
        )
        .await
        .expect_err("bound a taken port");
        let msg = e.to_string();
        assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse);
        assert!(msg.contains(&addr.to_string()), "{msg}");
        assert!(msg.contains("--addr"), "{msg}");
    }

    /// `serve` refuses a reachable address unless the server is public, before binding.
    #[tokio::test]
    async fn a_reachable_bind_needs_public() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = fixture("alpha", dir.path());
        let addr: std::net::SocketAddr = "0.0.0.0:0".parse().expect("addr");
        let e = crate::serve_in(
            vec![p],
            addr,
            None,
            gitterrace_config::Config::default(),
            None,
            None,
        )
        .await
        .expect_err("served without public");
        assert!(e.to_string().contains("--public"), "{e}");
    }
}
