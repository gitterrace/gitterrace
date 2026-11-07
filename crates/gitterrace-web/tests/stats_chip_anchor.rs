//! An author or path chip narrows the commits `/stats` counts, never the days its period means:
//! "last week" ends on the repository's newest commit, not on the chip's.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::Fixture;

/// Bob's last commit is ten days before the repository's newest one, Ann's.
fn router() -> (tempfile::TempDir, axum::Router) {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    let mut f = Fixture::init(&p);
    for (who, date) in [
        ("Bob <bob@y.io>", "2024-03-01T12:00:00+0000"),
        ("Bob <bob@y.io>", "2024-03-10T12:00:00+0000"),
        ("Ann <ann@x.io>", "2024-03-20T12:00:00+0000"),
    ] {
        f = f.at(date).write("a.txt", date);
        f.git(&["add", "-A"]);
        f.git(&["commit", "-q", "--author", who, "-m", "feat: n"]);
    }
    (root, gitterrace_web::app_router(&p).expect("router"))
}

async fn body(r: &axum::Router, uri: &str) -> String {
    gitterrace_testkit::wait::page_until(r, uri, |b| {
        b.contains("Commits per") || b.contains("No commits match")
    })
    .await
}

/// The bold "now" cell of the commits row.
fn commits(page: &str) -> Option<String> {
    let row = page.split("Commits without merges").nth(1)?;
    let b = row.split("<b>").nth(1)?;
    Some(b.split("</b>").next()?.to_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chip_keeps_the_repository_week() {
    let (_root, r) = router();
    let page = body(&r, "/stats?period=1w&author=bob%40y.io").await;
    // The week is 14 to 20 March: Bob has nothing in it.
    assert!(page.contains("No commits match"), "{page}");
    let page = body(&r, "/stats?period=1m&author=bob%40y.io").await;
    // 30 days ending 20 March start on 20 February: both of Bob's commits.
    assert_eq!(commits(&page).as_deref(), Some("2"), "{page}");
    assert!(page.contains("2024-02-20 to 2024-03-20"), "{page}");
}
