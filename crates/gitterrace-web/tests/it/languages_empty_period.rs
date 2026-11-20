//! A period that holds no commit of the first-parent line says so and still draws the chart.

use gitterrace_testkit::{Fixture, page as get};

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_period_shows_the_empty_card_and_the_chart() {
    let root = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(&root.path().join("repo"))
        .at("2020-01-01T12:00:00+00:00")
        .write("a.rs", "fn main() {}\n")
        .commit("feat: old")
        .branch("side")
        .at("2026-11-01T12:00:00+00:00")
        .write("b.rs", "fn b() {}\n")
        .commit("feat: new")
        .checkout("main")
        .at("2020-01-02T12:00:00+00:00");
    f.git(&["merge", "-q", "--no-ff", "side", "-m", "feat: merge"]);
    let router = gitterrace_web::app_router(f.path()).expect("router");
    let body = gitterrace_testkit::wait::page_until(
        &router,
        "/stats/languages?period=1w",
        gitterrace_testkit::wait::indexed,
    )
    .await;
    let (status, _) = get(&router, "/stats/languages?period=1w").await;
    assert_eq!(status, 200);
    assert!(body.contains("No commits in this period."), "{body}");
    assert!(body.contains("<svg"), "the chart is still drawn");
    let (_, all) = get(&router, "/stats/languages").await;
    assert!(!all.contains("No commits in this period."));
    // Every commit of the range list carries its short id: no empty chip.
    let list = all.split("class=\"rlist\"").nth(1).expect("the range list");
    assert!(list.contains("class=\"sha\""), "the list has chips");
    assert!(!list.contains("class=\"sha\" href=\"\""), "no chip without a target");
    assert!(!list.contains("></a>"), "no empty chip");
}
