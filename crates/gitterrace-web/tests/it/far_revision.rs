//! A revision too far from the indexed branch gets a page that says so, never a load of every
//! commit it lacks: the API answers `needs_reindex`, HEAD keeps serving from the index, and a
//! revision within the cap is served as usual.

use gitterrace_testkit::{chain, git, oracle, page};

/// `main` with a few commits, `near` 3 commits off it and `far` past the cap.
fn repo(dir: &std::path::Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    chain(dir, "main", None, 5);
    chain(dir, "near", Some("refs/heads/main"), 3);
    chain(dir, "far", Some("refs/heads/main"), gitterrace_index::feed::FRESH_CAP + 5);
    git(dir, &["checkout", "-q", "main"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_far_revision_asks_for_a_reindex_and_holds_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    repo(dir.path());
    let router = crate::common::ready_router(dir.path()).await;

    let (status, body) = page(&router, "/stats?rev=far").await;
    assert_eq!(status, 200);
    assert!(body.contains("need a reindex"), "far page missing: {body}");
    assert!(body.contains("indexed branch"), "{body}");
    assert!(!body.contains("data-progress"), "indexing page instead: {body}");

    let (status, api) = page(&router, "/api/v1/insights/summary?rev=far").await;
    assert_eq!(status, 409, "{api}");
    assert!(api.contains("\"needs_reindex\""), "{api}");

    // Other views of that commit keep working.
    let (status, _) = page(&router, "/log/far").await;
    assert_eq!(status, 200);

    // HEAD still reads the index, and a revision within the cap is served.
    let (status, head) = page(&router, "/stats").await;
    assert_eq!(status, 200);
    assert!(!head.contains("need a reindex") && !head.contains("data-progress"));
    oracle::ready_at(&router, "near").await;
    let (status, near) = page(&router, "/api/v1/insights/summary?rev=near").await;
    assert_eq!(status, 200, "{near}");

    // The cache still holds HEAD's history only: nothing was indexed for `far`.
    let (status, again) = page(&router, "/stats?rev=far").await;
    assert_eq!(status, 200);
    assert!(again.contains("need a reindex"), "{again}");
}
