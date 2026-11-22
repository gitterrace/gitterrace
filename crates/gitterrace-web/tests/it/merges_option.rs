//! The merges option is one switch for the whole stats page: the commits KPI, the per-day bars
//! and the punch card agree with it on and off, and lines and files never change.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::Fixture;

use crate::common::{bars, pulse};

fn router() -> (tempfile::TempDir, axum::Router) {
    let root = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(&root.path().join("repo"))
        .at("2024-03-01T12:00:00Z")
        .write("a.txt", "a\n")
        .commit("feat: first")
        .branch("side")
        .at("2024-03-02T12:00:00Z")
        .write("b.txt", "b\nb\nb\n")
        .commit("feat: side")
        .checkout("-")
        .at("2024-03-03T12:00:00Z")
        .write("c.txt", "c\n")
        .commit("fix: main");
    f.git(&["merge", "--no-ff", "-q", "-m", "merge side", "side"]);
    let r = gitterrace_web::app_router(f.path()).expect("router");
    (root, r)
}

async fn page(r: &axum::Router, uri: &str) -> String {
    gitterrace_testkit::wait::page_until(r, uri, |b| {
        b.contains("Commits per") && gitterrace_testkit::wait::indexed(b)
    })
    .await
}

/// The column total every punch card cell names ("of N").
fn punch(page: &str) -> u64 {
    let heat = page.split("class=\"chart heat\"").nth(1).expect("heat");
    let tip = heat.split("data-tip=\"").nth(1).expect("tip");
    let tip = tip.split('"').next().expect("end");
    tip.rsplit(" of ").next().expect("total").replace(',', "").parse().expect("number")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_follows_the_merges_option_and_lines_and_files_do_not() {
    let (_root, r) = router();
    let off = page(&r, "/stats?period=all").await;
    let on = page(&r, "/stats?period=all&merges=1").await;
    assert_eq!(pulse(&off, "Commits without merges"), 3);
    assert_eq!(pulse(&on, "Commits"), 4);
    for (p, n) in [(&off, 3), (&on, 4)] {
        assert_eq!(bars(p), n, "per-day bars");
        assert_eq!(punch(p), n, "punch card");
    }
    for label in ["Files changed", "Lines added", "Lines removed"] {
        assert_eq!(pulse(&off, label), pulse(&on, label), "{label}");
    }
    assert!(on.contains("merges=1"), "the link and the toggle carry it");
}

/// The overview counts every commit and the stats pages count commits without merges. Both
/// numbers are right, so each page must say which one it shows, and they must add up.
#[tokio::test(flavor = "multi_thread")]
async fn overview_total_is_stats_total_plus_merges_and_both_say_so() {
    let (_root, r) = router();
    let stats = page(&r, "/stats").await;
    let (_, overview) = gitterrace_testkit::page(&r, "/").await;
    let shown_all = pulse(&overview, "class=\"facts\"");
    assert_eq!(shown_all, pulse(&stats, "Commits without merges") + 1, "overview = stats + merges");
    assert!(overview.contains("1 merge"), "overview names its merges");
    assert!(stats.contains("Merges are left out"), "stats says it leaves merges out");
}
