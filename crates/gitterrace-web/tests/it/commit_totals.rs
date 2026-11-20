//! The overview counts every commit and the stats pages count commits without merges. Both
//! numbers are right, so each page must say which one it shows, and they must add up.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, page as get};

/// The number inside the first `<b>` after `marker`, separators gone.
fn first_bold(html: &str, marker: &str) -> u64 {
    let rest = html.split(marker).nth(1).expect("marker");
    let cell = rest.split("<b").nth(1).expect("bold");
    let digits: String = cell
        .split_once('>')
        .expect("tag")
        .1
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().expect("number")
}

#[tokio::test(flavor = "multi_thread")]
async fn overview_total_is_stats_total_plus_merges_and_both_say_so() {
    let root = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(&root.path().join("repo"))
        .at("2024-03-01T12:00:00Z")
        .write("a.txt", "a\n")
        .commit("feat: first")
        .branch("side")
        .at("2024-03-02T12:00:00Z")
        .write("b.txt", "b\n")
        .commit("feat: side")
        .checkout("-")
        .at("2024-03-03T12:00:00Z")
        .write("c.txt", "c\n")
        .commit("fix: main");
    f.git(&["merge", "--no-ff", "-q", "-m", "merge side", "side"]);
    let count = |args: &[&str]| -> u64 { f.git(args).trim().parse().expect("n") };
    let merges = count(&["rev-list", "--count", "--merges", "HEAD"]);
    let all = count(&["rev-list", "--count", "HEAD"]);
    assert_eq!((all, merges), (4, 1));

    let router = gitterrace_web::app_router(f.path()).expect("router");
    gitterrace_testkit::wait::page_until(&router, "/stats", gitterrace_testkit::wait::indexed)
        .await;
    let (_, overview) = get(&router, "/").await;
    let (_, stats) = get(&router, "/stats").await;

    let shown_all = first_bold(&overview, "class=\"facts\"");
    let shown_stats = first_bold(&stats, "Commits without merges</th>");
    assert_eq!(shown_all, shown_stats + merges, "overview = stats + merges");
    assert!(overview.contains(&format!("{merges} merge")), "overview names its merges");
    assert!(
        stats.contains("Commits without merges") && stats.contains("Merges are left out"),
        "stats says it leaves merges out"
    );
}
