//! HTTP server: server-rendered pages over `gitterrace-git` and `gitterrace-index`.

mod app;
mod cards;
mod charts;
mod error;
mod filter;
mod fmt;
mod guard;
mod insights;
mod languages;
mod markdown;
mod media;
mod multi;
mod pages;
pub mod palette;
mod routes;
mod search;
mod types;
mod views;

use std::{
    io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use gitterrace_git::Repo;
use gitterrace_index::Index;
pub use multi::discover_repos;

use crate::app::App;

/// The full router `serve` uses: pages, `/api/v1`, cards and badges. Starts the index
/// refresher, so call it inside a tokio runtime. Drive it in-process with tower's `oneshot`.
///
/// Requests must carry a loopback `Host` header (`localhost`, `127.0.0.1`, `[::1]`), which
/// stops DNS-rebinding pages from reading the repository. In-process callers send
/// `Host: localhost`.
pub fn app_router(repo_path: impl AsRef<Path>) -> io::Result<axum::Router> {
    Ok(build(repo_path.as_ref(), None)?.0)
}

/// Router for several repositories: an index at `/`, each repository under `/r/NAME`.
/// One path behaves exactly like [`app_router`]. Same Host rules.
pub fn multi_router(paths: Vec<PathBuf>) -> io::Result<axum::Router> {
    Ok(build_multi(paths, None)?.0)
}

/// One repository's pages, `/api/v1`, cards and badges, with links under `base`. Starts its
/// index refresher, so call it inside a tokio runtime.
fn repo_router(repo_path: &Path, base: String) -> io::Result<axum::Router> {
    let repo = Repo::discover(repo_path).map_err(io::Error::other)?;
    let index = Index::open(&repo)?;
    let mut app = App::new(repo, index);
    app.base = base;
    let app = Arc::new(app);
    tokio::spawn(app::keep_index_fresh(Arc::clone(&app)));
    let api = gitterrace_api::ApiState::open(repo_path).map_err(io::Error::other)?;
    let pages = routes::router(Arc::clone(&app)).nest("/api/v1", gitterrace_api::router(api));
    tokio::spawn(prewarm(Arc::clone(&app), pages.clone()));
    Ok(pages)
}

/// Unfiltered pages rendered in the background whenever HEAD moves. Each costs 40-90 ms to
/// build on a 14k-commit repo, then a few ms from the memo, so no visitor pays the build.
const WARM_PAGES: [&str; 11] = [
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

/// Parses the record set for HEAD as soon as each index generation is ready, then renders
/// the unfiltered pages once, so the first visit to any of them is served from the memo.
async fn prewarm(app: Arc<App>, pages: axum::Router) {
    use tower::ServiceExt;
    let mut warmed: Option<String> = None;
    loop {
        let head = app::blocking(&app, |a| {
            let head = a.repo.resolve("HEAD")?;
            a.records(&head)?;
            Ok(head)
        })
        .await
        .ok();
        if head.is_some() && head == warmed {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        } else if head.is_some() && app.ready() {
            for path in WARM_PAGES {
                // Internal requests skip the Host and cross-site guard, which wraps the outer
                // router; a failed warm-up only means the first visitor builds the page.
                if let Ok(req) = axum::http::Request::get(path).body(axum::body::Body::empty()) {
                    let _warm = pages.clone().oneshot(req).await;
                }
            }
            warmed = head;
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
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

fn build(repo_path: &Path, extra: Option<IpAddr>) -> io::Result<(axum::Router, String)> {
    let name = Repo::discover(repo_path).map_err(io::Error::other)?.name();
    Ok((guarded(repo_router(repo_path, String::new())?, extra), name))
}

fn build_multi(paths: Vec<PathBuf>, extra: Option<IpAddr>) -> io::Result<(axum::Router, String)> {
    match <[PathBuf; 1]>::try_from(paths) {
        Ok([one]) => build(&one, extra),
        Err(paths) => {
            let n = paths.len();
            let table = Arc::new(multi::Table::new(paths)?);
            Ok((
                guarded(multi::router(table), extra),
                format!("{n} repositories"),
            ))
        }
    }
}

/// Starts serving until ctrl-c.
pub async fn serve(repo_paths: Vec<PathBuf>, addr: SocketAddr) -> io::Result<()> {
    let extra = (!addr.ip().is_loopback()).then(|| addr.ip());
    let (router, name) = build_multi(repo_paths, extra)?;
    if !addr.ip().is_loopback() {
        tracing::warn!(
            "serving to the network: anyone who can reach this address can read the repository"
        );
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("serving {name} on http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
        })
        .await
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::{process::Command, sync::Arc};

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::{app::App, routes};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@e.x"])
            .args(args)
            .current_dir(dir)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    async fn get(app: &Arc<App>, uri: &str) -> (u16, String) {
        let res = routes::router(Arc::clone(app))
            .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = res.status().as_u16();
        let bytes = res.into_body().collect().await.expect("body").to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
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
        assert_eq!((s, body.contains("fn main")), (200, true));
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
        let big = "x\n".repeat(crate::views::MAX_BLOB_BYTES as usize / 2 + 10);
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
        assert!(body.contains("<object"));
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
    async fn app_router_nests_api_cards_and_badges() {
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
        assert_eq!(status("/badges/nope.svg").await, 404);
        assert_eq!(status("/cards/wrapped/x.svg").await, 404);
        assert!(matches!(status("/badges/commits.svg").await, 200 | 503));
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
        for (host, want) in [
            (Some("evil.example:4242"), 403),
            (None, 403),
            (Some("localhost"), 200),
            (Some("127.0.0.1:4242"), 200),
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
        for uri in ["/", "/badges/commits.svg", "/cards/wrapped/x.svg"] {
            let res = send(uri, Some("cross-site")).await.expect("response");
            assert_eq!(res.status().as_u16(), 403, "{uri}");
            assert_eq!(res.headers()["cross-origin-resource-policy"], "same-origin");
        }
        for site in [Some("same-origin"), Some("same-site"), Some("none"), None] {
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
        let p = root.join(name);
        std::fs::create_dir_all(p.join("src")).expect("dir");
        git(&p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("README.md"), "# Hi\n\n[a](src/a.rs) ![i](img.png)\n").expect("w");
        std::fs::write(p.join("src/a.rs"), "fn main() {}\n").expect("w");
        git(&p, &["add", "."]);
        git(&p, &["commit", "-q", "-m", "feat(core): first"]);
        std::fs::write(p.join("src/a.rs"), "fn main() {\n}\n").expect("w");
        git(&p, &["commit", "-qam", "fix: second"]);
        p
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
                    out.push(v.replace("&amp;", "&"));
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
            assert_eq!(fetch(&router, u, "evil.example:4242").await.0, 403, "{u}");
        }
        assert_eq!(
            fetch(&router, "/r/alpha/api/v1/repo", "127.0.0.1:4242")
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
        let t = crate::multi::Table::new(vec![a, b]).expect("table");
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
}
