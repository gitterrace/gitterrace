//! The contributors page renders the top rows and links to the full list; its KPI and the API
//! still count everyone.

use gitterrace_testkit::{Fixture, page as get};

#[tokio::test(flavor = "multi_thread")]
async fn contributors_page_caps_rows_and_links_to_all() {
    let root = tempfile::tempdir().expect("tmp");
    let mut f = Fixture::init(root.path());
    for i in 0..105 {
        f = f.write("f.txt", &format!("{i}\n")).commit_by(&format!("P{i} <p{i}@e.x>"), "feat: x");
    }
    let router = gitterrace_web::app_router(root.path()).expect("router");
    let page = gitterrace_testkit::wait::page_until(
        &router,
        "/stats/contributors",
        gitterrace_testkit::wait::indexed,
    )
    .await;
    let rows = |html: &str| html.matches("class=\"av ").count();
    assert_eq!(rows(&page), 100, "the page renders the top rows only");
    assert!(page.contains("Show all 105 contributors"));
    assert!(page.contains("all=1"));
    assert!(page.contains(">105<"), "the KPI counts everyone");
    let (_, all) = get(&router, "/stats/contributors?all=1").await;
    assert_eq!(rows(&all), 105);
    assert!(!all.contains("Show all"));
}
