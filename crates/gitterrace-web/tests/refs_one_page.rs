//! Branches & tags is one page: no tab row, sections in a fixed order, empty ones left out.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, oracle, page};
use tower::ServiceExt;

async fn location(router: &axum::Router, uri: &str) -> (u16, String) {
    let req = axum::http::Request::get(uri)
        .header("host", "localhost")
        .body(axum::body::Body::empty())
        .expect("request");
    let res = router.clone().oneshot(req).await.expect("answer");
    let loc =
        res.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
    (res.status().as_u16(), loc)
}

#[tokio::test]
async fn refs_is_one_page_with_ordered_sections() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path()).write("a.rs", "fn main() {}\n").commit("feat: first");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    oracle::ready(&router).await;

    let (status, body) = page(&router, "/refs").await;
    assert_eq!(status, 200);
    assert!(!body.contains("<nav class=\"tabs\">"), "no sub-tab row");
    assert!(body.contains(">Branches<"), "Branches always shows");
    for gone in ["Tags", "Worktrees", "Stashes", "Remotes"] {
        assert!(!body.contains(&format!("<h2 id=\"{}\"", gone.to_lowercase())), "{gone}");
    }

    f.git(&["tag", "v1"]);
    let f = f.write("b.txt", "wip\n");
    f.git(&["add", "b.txt"]);
    f.git(&["stash"]);
    let wt = dir.path().join("wt");
    f.git(&["worktree", "add", "-q", wt.to_str().expect("utf8"), "-b", "side"]);
    let (_, body) = page(&router, "/refs").await;
    let at = |id: &str| body.find(&format!("<h2 id=\"{id}\"")).unwrap_or(usize::MAX);
    let order = ["branches", "tags", "worktrees", "stashes"].map(at);
    assert!(order.iter().all(|&i| i != usize::MAX), "{order:?}");
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert!(!body.contains("<nav class=\"tabs\">"));

    let root = gitterrace_testkit::base(&router).await;
    for (old, to) in [("/stashes", "stashes"), ("/worktrees", "worktrees")] {
        assert_eq!(
            location(&router, &format!("{root}{old}")).await,
            (308, format!("{root}/refs#{to}")),
            "{old}"
        );
    }
}
