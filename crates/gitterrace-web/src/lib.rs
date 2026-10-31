//! HTTP server: server-rendered pages over `gitterrace-git` and `gitterrace-index`.

mod app;
mod cards;
mod charts;
mod error;
mod filter;
mod fmt;
mod languages;
mod markdown;
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

use axum::response::IntoResponse;
use gitterrace_git::Repo;
use gitterrace_index::Index;

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

/// Host-header allowlist: loopback names plus, when bound elsewhere, the bound IP.
fn host_allowed(host: &str, extra: Option<IpAddr>) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((ip, _)) => ip,
            None => return false,
        },
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    let name = name.to_ascii_lowercase();
    if matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return true;
    }
    extra.is_some_and(|ip| name.parse::<IpAddr>().is_ok_and(|n| n == ip))
}

async fn host_guard(
    axum::extract::State(extra): axum::extract::State<Option<IpAddr>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let ok = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| host_allowed(h, extra));
    if ok {
        next.run(req).await
    } else {
        (
            axum::http::StatusCode::FORBIDDEN,
            "forbidden: unrecognised Host header\n",
        )
            .into_response()
    }
}

fn build(repo_path: &Path, extra: Option<IpAddr>) -> io::Result<(axum::Router, String)> {
    let repo = Repo::discover(repo_path).map_err(io::Error::other)?;
    let index = Index::open(&repo)?;
    let app = Arc::new(App::new(repo, index));
    tokio::spawn(app::keep_index_fresh(Arc::clone(&app)));
    let api = gitterrace_api::ApiState::open(repo_path).map_err(io::Error::other)?;
    let name = app.name.clone();
    let router = routes::router(app)
        .nest("/api/v1", gitterrace_api::router(api))
        .layer(axum::middleware::from_fn_with_state(extra, host_guard));
    Ok((router, name))
}

/// Starts serving until ctrl-c.
pub async fn serve(repo_path: PathBuf, addr: SocketAddr) -> io::Result<()> {
    let extra = (!addr.ip().is_loopback() && !addr.ip().is_unspecified()).then(|| addr.ip());
    let (router, name) = build(&repo_path, extra)?;
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
        assert!(body.contains("<h1 id=\"hello"));
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

    #[test]
    fn host_allowlist() {
        let ip: std::net::IpAddr = "192.168.1.5".parse().expect("ip");
        for h in [
            "localhost",
            "localhost:4242",
            "127.0.0.1:4242",
            "[::1]:4242",
            "LOCALHOST",
        ] {
            assert!(crate::host_allowed(h, None), "{h}");
        }
        for h in ["evil.example:4242", "192.168.1.5:80", "[::1", ""] {
            assert!(!crate::host_allowed(h, None), "{h}");
        }
        assert!(crate::host_allowed("192.168.1.5:4242", Some(ip)));
        assert!(!crate::host_allowed("10.0.0.1", Some(ip)));
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
    }
}
