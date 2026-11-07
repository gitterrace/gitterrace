//! The merges option is one switch for the whole stats page: the commits KPI, the per-day bars
//! and the punch card agree with it on and off, and lines and files never change.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::time::Duration;

use gitterrace_testkit::{Fixture, page as get};

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
    let mut b = String::new();
    for _ in 0..200 {
        b = get(r, uri).await.1;
        if b.contains("Commits per") && !b.contains("data-progress") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    b
}

/// The first number in the `<b>` of the pulse row `label`, whose label cell is a row header.
fn row(page: &str, label: &str) -> u64 {
    let cell = [format!("{label}</th>"), format!("{label}</td>")]
        .iter()
        .find_map(|end| page.split(end.as_str()).nth(1))
        .expect("row")
        .split("<b>")
        .nth(1)
        .expect("value");
    cell.split("</b>")
        .next()
        .expect("end")
        .replace(',', "")
        .parse()
        .expect("number")
}

fn bars(page: &str) -> u64 {
    page.split("aria-label=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .filter_map(|l| l.split(": ").nth(1)?.split(' ').next()?.parse::<u64>().ok())
        .sum()
}

/// The column total every punch card cell names ("of N").
fn punch(page: &str) -> u64 {
    let heat = page.split("class=\"chart heat\"").nth(1).expect("heat");
    let tip = heat.split("data-tip=\"").nth(1).expect("tip");
    let tip = tip.split('"').next().expect("end");
    tip.rsplit(" of ")
        .next()
        .expect("total")
        .replace(',', "")
        .parse()
        .expect("number")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_follows_the_merges_option_and_lines_and_files_do_not() {
    let (_root, r) = router();
    let off = page(&r, "/stats?period=all").await;
    let on = page(&r, "/stats?period=all&merges=1").await;
    assert_eq!(row(&off, "Commits without merges"), 3);
    assert_eq!(row(&on, "Commits"), 4);
    for (p, n) in [(&off, 3), (&on, 4)] {
        assert_eq!(bars(p), n, "per-day bars");
        assert_eq!(punch(p), n, "punch card");
    }
    for label in ["Files changed", "Lines added", "Lines removed"] {
        assert_eq!(row(&off, label), row(&on, label), "{label}");
    }
    assert!(on.contains("merges=1"), "the link and the toggle carry it");
}
