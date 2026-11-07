//! A period that holds no commit of the first-parent line says so and still draws the chart.

use gitterrace_testkit::{git, git_at, page as get};

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_period_shows_the_empty_card_and_the_chart() {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.rs"), "fn main() {}\n").expect("w");
    git(&p, &["add", "."]);
    git_at(
        &p,
        "2020-01-01T12:00:00+00:00",
        &["commit", "-q", "-m", "feat: old"],
    );
    git(&p, &["checkout", "-q", "-b", "side"]);
    std::fs::write(p.join("b.rs"), "fn b() {}\n").expect("w");
    git(&p, &["add", "."]);
    git_at(
        &p,
        "2026-11-01T12:00:00+00:00",
        &["commit", "-q", "-m", "feat: new"],
    );
    git(&p, &["checkout", "-q", "main"]);
    git_at(
        &p,
        "2020-01-02T12:00:00+00:00",
        &["merge", "-q", "--no-ff", "side", "-m", "feat: merge"],
    );

    let router = gitterrace_web::app_router(&p).expect("router");
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
    assert!(
        !list.contains("class=\"sha\" href=\"\""),
        "no chip without a target"
    );
    assert!(!list.contains("></a>"), "no empty chip");
}
