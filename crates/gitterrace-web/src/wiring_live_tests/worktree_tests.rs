//! Wiring tests for the working tree routes: which paths they answer, the image view and
//! the raw budget.

use std::{sync::Arc, time::Duration};

use axum::{Router, body::Body, http::Request};
use gitterrace_config::Config;
use gitterrace_testkit::page as get;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::wiring_tests::{app, repo, router, with};

/// The folded-file route of the working tree answers changed files only: an ignored file and
/// `.git/config` both exist on disk and still get the unchanged-file answer.
#[tokio::test]
async fn worktree_fragments_refuse_ignored_and_git_paths() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    std::fs::write(p.join(".git/info/exclude"), "*.env\n").expect("exclude");
    std::fs::write(p.join("prod.env"), "SECRET=hunter2\n").expect("write");
    std::fs::write(p.join("small.txt"), "changed now\n").expect("write");
    let r = router(&p, Config::default());
    let (_, ok) = get(&r, "/worktree/diff/small.txt").await;
    assert!(ok.contains("changed now"), "{ok}");
    for bad in ["prod.env", ".git/config", "d/../small.txt"] {
        let (_, body) = get(&r, &format!("/worktree/diff/{bad}")).await;
        assert!(
            !body.contains("SECRET") && !body.contains("[core]"),
            "{bad}: {body}"
        );
        assert!(body.contains("No longer changed"), "{bad}: {body}");
    }
}

/// A changed image on the working tree page shows HEAD and the copy on disk, the latter
/// through `/worktree/raw`, which serves it with `/raw`'s content type and never caches it.
#[tokio::test]
async fn a_changed_image_in_the_working_tree_shows_both_sides() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let png = |w: u8| {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend([0, 0, 0, w, 0, 0, 0, 5, 8, 6, 0, 0, 0, 0]);
        b
    };
    std::fs::write(p.join("a.png"), png(7)).expect("write");
    gitterrace_testkit::git(&p, &["add", "a.png"]);
    gitterrace_testkit::git(&p, &["commit", "-qm", "feat: image"]);
    std::fs::write(p.join("a.png"), png(9)).expect("write");
    let r = router(&p, Config::default());
    let (_, page) = get(&r, "/worktree").await;
    let at = page.find("data-path=\"a.png\"").expect("a.png section");
    let sect = &page[at..at + page[at..].find("</section>").expect("end")];
    assert!(!sect.contains("Binary file not shown"), "{sect}");
    // Both sides load through the repository prefix the server mounts it under.
    assert!(sect.contains("src=\"/r/r/raw/HEAD/a.png\""), "{sect}");
    assert!(sect.contains("src=\"/r/r/worktree/raw/a.png\""), "{sect}");
    let res = r
        .clone()
        .oneshot(
            axum::http::Request::get("/r/r/worktree/raw/a.png")
                .header("host", gitterrace_testkit::DEFAULT_ADDR)
                .body(axum::body::Body::empty())
                .expect("req"),
        )
        .await
        .expect("res");
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(res.headers()["cache-control"], "no-store");
}

/// `/worktree/raw` serves changed files only: an ignored file, `.git/config`, a `..` path and
/// a clean tracked file all answer 404 with none of their bytes; a public server has no route.
#[tokio::test]
async fn worktree_raw_refuses_ignored_git_and_clean_paths() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    std::fs::write(p.join(".git/info/exclude"), "*.env\n").expect("exclude");
    std::fs::write(p.join("prod.env"), "SECRET=hunter2\n").expect("write");
    std::fs::write(p.join("small.txt"), "changed now\n").expect("write");
    let r = router(&p, Config::default());
    let (s, ok) = get(&r, "/worktree/raw/small.txt").await;
    assert_eq!((s, ok.as_str()), (200, "changed now\n"));
    for bad in ["prod.env", ".git/config", "d/../small.txt", "mid.txt"] {
        let (s, body) = get(&r, &format!("/worktree/raw/{bad}")).await;
        assert_eq!(s, 404, "{bad}: {body}");
        assert!(
            !body.contains("SECRET") && !body.contains("[core]") && !body.contains("needle"),
            "{bad}: {body}"
        );
    }
    let public = router(&p, with(|c| c.server.is_public = true));
    assert_eq!(get(&public, "/worktree/raw/small.txt").await.0, 404);
}

/// An untracked file opens in the blob view from disk, linked from the working tree page;
/// a file that is not changed and a public server both answer not found.
#[tokio::test]
async fn worktree_blob_shows_an_untracked_file_and_hides_it_on_a_public_server() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    std::fs::write(p.join("fresh.rs"), "fn fresh_marker() {}\n").expect("write");
    let r = router(&p, Config::default());
    let (s, body) = get(&r, "/worktree/blob/fresh.rs").await;
    assert_eq!(s, 200, "{body}");
    assert!(body.contains("fresh_marker"), "{body}");
    assert!(body.contains("id=\"L1\""), "line anchors: {body}");
    let (_, list) = get(&r, "/worktree").await;
    assert!(list.contains("/worktree/blob/fresh.rs\""), "{list}");
    for bad in [".git/config", "mid.txt", "d/../fresh.rs"] {
        assert_eq!(
            get(&r, &format!("/worktree/blob/{bad}")).await.0,
            404,
            "{bad}"
        );
    }
    let public = router(&p, with(|c| c.server.is_public = true));
    assert_eq!(get(&public, "/worktree/blob/fresh.rs").await.0, 404);
}

/// `/worktree/raw` takes its share of `limits.raw_concurrent_mb` like `/raw`: two unread
/// bodies fill the budget and a third reader waits until one is sent.
#[tokio::test]
async fn worktree_raw_holds_the_raw_budget_until_a_body_is_sent() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    std::fs::write(p.join("w.txt"), "w".repeat(4096)).expect("write");
    let a = app(
        &p,
        with(|c| {
            c.limits.raw_mb = 1;
            c.limits.raw_concurrent_mb = 2;
        }),
    );
    let r = crate::routes::router(Arc::clone(&a));
    let send = |r: Router| async move {
        let req = Request::get("/worktree/raw/w.txt")
            .body(Body::empty())
            .expect("req");
        r.oneshot(req).await.expect("response")
    };
    let first = send(r.clone()).await;
    let second = send(r.clone()).await;
    assert_eq!(first.status(), 200);
    assert_eq!(
        a.env.raw_budget.available_permits(),
        0,
        "two readers fill it"
    );
    let third = tokio::spawn(send(r.clone()));
    gitterrace_testkit::wait::pause(Duration::from_millis(100)).await;
    assert!(!third.is_finished(), "the third waits before reading");
    let bytes = first.into_body().collect().await.expect("body").to_bytes();
    assert_eq!(bytes.len(), 4096, "the body is the whole file");
    let third = tokio::time::timeout(Duration::from_secs(5), third)
        .await
        .expect("served once a body is sent")
        .expect("task");
    assert_eq!(third.status(), 200);
    drop((second, third));
    assert_eq!(
        a.env.raw_budget.available_permits(),
        2,
        "every share comes back"
    );
}

/// A file `.gitattributes` at HEAD marks generated folds on the working tree page as on a
/// commit page, and an uncommitted edit to `.gitattributes` moves no fold.
#[tokio::test]
async fn generated_files_fold_in_the_working_tree_by_the_attributes_at_head() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    std::fs::write(p.join(".gitattributes"), "gen.json linguist-generated\n").expect("write");
    std::fs::write(p.join("gen.json"), "{}\n").expect("write");
    std::fs::write(p.join("hand.rs"), "fn a() {}\n").expect("write");
    gitterrace_testkit::git(&p, &["add", "."]);
    gitterrace_testkit::git(&p, &["commit", "-qm", "feat: generated"]);
    std::fs::write(p.join("gen.json"), "{\"a\": 1}\n").expect("write");
    std::fs::write(p.join("hand.rs"), "fn b() {}\n").expect("write");
    // Uncommitted: marks hand.rs and unmarks gen.json, and neither may take effect.
    std::fs::write(p.join(".gitattributes"), "hand.rs linguist-generated\n").expect("write");
    let r = router(&p, Config::default());
    let (_, page) = get(&r, "/worktree").await;
    let sect = |path: &str| {
        let at = page
            .find(&format!("data-path=\"{path}\""))
            .unwrap_or_else(|| panic!("{path} section"));
        page[at..at + page[at..].find("</section>").expect("end")].to_owned()
    };
    let generated = sect("gen.json");
    assert!(generated.contains("class=\"tag\">Generated"), "{generated}");
    assert!(
        generated.contains("data-load-diff=\"/r/r/worktree/diff/gen.json\""),
        "{generated}"
    );
    assert!(!generated.contains("\"a\": 1"), "{generated}");
    let hand = sect("hand.rs");
    assert!(!hand.contains("Generated"), "{hand}");
    assert!(hand.contains("class=\"diffwrap\""), "{hand}");
}
