//! Tests for the server: every page, the routers, the Host guard and the public mode.

use gitterrace_testkit::{Fixture, git};
use std::sync::Arc;

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::{app::App, routes};

pub(crate) async fn get(app: &Arc<App>, uri: &str) -> (u16, String) {
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
    assert!(body.contains("<h1 id=\"hello"));
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
        body.contains("src/a.rs") && body.contains("new.txt") && body.contains("class=\"diff\"")
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
        gitterrace_config::Config::default().limits.preview_mb as usize,
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
    assert!(body.contains("class=\"media-img\" src=\"/raw/HEAD/a.png\"") && body.contains("7 x 5"));
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

pub(crate) fn fixture(name: &str, root: &std::path::Path) -> std::path::PathBuf {
    Fixture::standard(&root.join(name)).path().to_owned()
}

pub(crate) async fn fetch(router: &axum::Router, uri: &str, host: &str) -> (u16, String) {
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
pub(crate) fn links(html: &str) -> Vec<String> {
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
