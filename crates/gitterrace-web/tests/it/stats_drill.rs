//! A bar on `/stats` links to the date range of its bucket, the range chip removes it again,
//! and the page without a range carries no range markup.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::Fixture;

use crate::common::{bars, pulse};

/// Week of Monday 2024-03-04: two commits on Wednesday, one on Thursday. Others are lone.
const DATES: [&str; 6] = [
    "2024-02-14T12:00:00+0000",
    "2024-02-28T12:00:00+0000",
    "2024-03-06T10:00:00+0000",
    "2024-03-06T14:00:00+0000",
    "2024-03-07T12:00:00+0000",
    "2024-03-20T12:00:00+0000",
];

fn router() -> (tempfile::TempDir, axum::Router) {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    let mut f = Fixture::init(&p);
    for (i, d) in DATES.iter().enumerate() {
        f = f.at(d).write("a.txt", &format!("{i}\n")).commit(&format!("feat: n{i}"));
    }
    (root, gitterrace_web::app_router(&p).expect("router"))
}

async fn body(r: &axum::Router, uri: &str) -> String {
    gitterrace_testkit::wait::page_until(r, uri, |b| {
        b.contains("Commits per") && gitterrace_testkit::wait::identified(b)
    })
    .await
}

/// The `href` of the bar link whose label says `needle`.
fn bar_href(page: &str, needle: &str) -> String {
    let at = page.find(&format!("aria-label=\"{needle}")).expect("bar link");
    let tag = &page[page[..at].rfind("<a ").expect("a")..at];
    let h = tag.split("href=\"").nth(1).expect("href");
    h.split('"').next().expect("end").replace("&amp;", "&")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bar_link_narrows_to_its_bucket() {
    let (_root, r) = router();
    let page = body(&r, "/stats?period=1y").await;
    let href = bar_href(&page, "4 Mar: 3 commits");
    assert!(
        href.contains("q=after%3A2024-03-04+before%3A2024-03-11")
            || href.contains("q=after%3A2024-03-04%20before%3A2024-03-11"),
        "the range is two terms of q, the end day exclusive: {href}"
    );
    assert!(!href.contains("from=") && !href.contains("to="), "links write q only: {href}");
    let href = href.replacen("/r/repo", "", 1);
    let narrowed = body(&r, &href).await;
    assert!(narrowed.contains("Wednesday: 2"), "{narrowed}");
    assert!(narrowed.contains("Thursday: 1"));
    assert!(narrowed.contains("Monday: 0"));
    assert!(narrowed.contains("2024-03-04 to 2024-03-10: 3 commits"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_chip_removes_the_range() {
    let (_root, r) = router();
    let page = body(&r, "/stats?period=1y&from=2024-03-04&to=2024-03-10").await;
    // The old keys and the new text read the same page.
    let by_q = body(&r, "/stats?q=after%3A2024-03-04%20before%3A2024-03-11").await;
    assert_eq!(pulse(&by_q, "Commits without merges"), pulse(&page, "Commits without merges"));
    let chips = page
        .split("<div class=\"chips\">")
        .nth(1)
        .and_then(|s| s.split("</div>").next())
        .expect("chips");
    assert!(chips.contains("<i>after</i> 2024-03-04"), "{chips}");
    assert!(chips.contains("<i>before</i> 2024-03-11"), "{chips}");
    // Each bound removes itself: dropping one leaves no range, so no range card.
    let remove =
        chips.split("href=\"").skip(1).find_map(|s| s.split('"').next()).expect("a chip link");
    let remove = remove.replacen("/r/repo", "", 1).replace("&amp;", "&");
    let back = body(&r, &remove).await;
    assert!(!back.contains("<h2>Commits by weekday</h2>"));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_range_there_is_no_range_markup() {
    let (_root, r) = router();
    let page = body(&r, "/stats?period=1y").await;
    assert!(!page.contains("<h2>Commits by weekday</h2>"));
    let bad = body(&r, "/stats?period=1y&from=nonsense&to=2024-03-10").await;
    assert_eq!(bad, page, "a malformed range is ignored");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_section_of_a_narrowed_page_covers_the_range() {
    let (_root, r) = router();
    let wide = body(&r, "/stats?period=1y").await;
    let narrow = body(&r, "/stats?period=1y&from=2024-03-04&to=2024-03-10").await;
    assert_eq!(pulse(&narrow, "Commits without merges"), 3, "{narrow}");
    assert_eq!(
        pulse(&narrow, "Commits without merges"),
        bars(&narrow),
        "the table and the per-day chart count the same commits"
    );
    assert!(pulse(&wide, "Commits without merges") > pulse(&narrow, "Commits without merges"));
    let span = narrow
        .split("<p class=\"hint\">")
        .nth(1)
        .and_then(|s| s.split("</p>").next())
        .expect("span line");
    assert!(span.contains("2024-03-04") && span.contains("2024-03-10"), "{span}");
    assert!(narrow.contains("Commits per day"), "{narrow}");
}
