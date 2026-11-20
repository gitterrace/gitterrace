//! An insights page whose filters match no commit says so. It never shows a date from an
//! empty set (1970-01-01) or charts drawn from nothing.

use gitterrace_testkit::{Fixture, page as get};

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_selection_reads_as_empty_on_every_insights_page() {
    let root = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(&root.path().join("repo"))
        .write("a.rs", "fn main() {}\n")
        .commit("feat: first");

    let router = gitterrace_web::app_router(f.path()).expect("router");
    // Commit search answers from the index, so a hit means the index is built.
    gitterrace_testkit::wait::page_until(&router, "/search?q=first&kind=commits", |b| {
        b.contains("class=\"crow\"")
    })
    .await;
    for page in [
        "/stats",
        "/stats/hotspots",
        "/stats/languages",
        "/stats/contributors",
    ] {
        let url = format!("{page}?q=author:nobody%40example.com");
        let (status, body) = get(&router, &url).await;
        assert_eq!(status, 200, "{url}");
        assert!(
            !body.contains("1970"),
            "{url} shows a date from an empty set"
        );
    }
}
