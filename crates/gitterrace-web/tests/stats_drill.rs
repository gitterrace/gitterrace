//! A bar on `/stats` links to the date range of its bucket, the range chip removes it again,
//! and the page without a range carries no range markup.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::Fixture;

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
        f = f
            .at(d)
            .write("a.txt", &format!("{i}\n"))
            .commit(&format!("feat: n{i}"));
    }
    (root, gitterrace_web::app_router(&p).expect("router"))
}

async fn body(r: &axum::Router, uri: &str) -> String {
    gitterrace_testkit::wait::page_until(r, uri, |b| b.contains("Commits per")).await
}

/// The `href` of the bar link whose label says `needle`.
fn bar_href(page: &str, needle: &str) -> String {
    let at = page
        .find(&format!("aria-label=\"{needle}"))
        .expect("bar link");
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
        href.contains("from=2024-03-04") && href.contains("to=2024-03-10"),
        "{href}"
    );
    assert!(href.contains("period=1y"), "filters carry over: {href}");
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
    let chips = page
        .split("<div class=\"chips\">")
        .nth(1)
        .and_then(|s| s.split("</div>").next())
        .expect("chips");
    assert!(
        chips.contains("<i>range</i> 2024-03-04 to 2024-03-10"),
        "{chips}"
    );
    let remove = chips
        .split("href=\"")
        .filter_map(|s| s.split('"').next())
        .find(|h| !h.contains("from=") && h.contains("period=1y"))
        .expect("a chip link without the range");
    assert!(!remove.contains("to="), "{remove}");
    let remove = remove.replacen("/r/repo", "", 1);
    let remove = remove.as_str();
    let back = body(&r, remove).await;
    assert!(!back.contains("<h2>Commits by weekday</h2>") && !back.contains("<i>range</i>"));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_range_there_is_no_range_markup() {
    let (_root, r) = router();
    let page = body(&r, "/stats?period=1y").await;
    assert!(!page.contains("<h2>Commits by weekday</h2>") && !page.contains("<i>range</i>"));
    let bad = body(&r, "/stats?period=1y&from=nonsense&to=2024-03-10").await;
    assert_eq!(bad, page, "a malformed range is ignored");
}

/// The commit total of the pulse table.
fn kpi_commits(page: &str) -> u64 {
    let row = page
        .split("Commits without merges</th>")
        .nth(1)
        .expect("commits row");
    let b = row.split("<b>").nth(1).expect("value");
    b.split("</b>")
        .next()
        .expect("end")
        .replace(',', "")
        .parse()
        .expect("number")
}

/// The commits of every bar of the per-bucket chart, summed.
fn bar_commits(page: &str) -> u64 {
    page.split("aria-label=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .filter_map(|l| {
            let n = l.split(": ").nth(1)?;
            n.split(' ').next()?.parse::<u64>().ok()
        })
        .sum()
}

#[tokio::test(flavor = "multi_thread")]
async fn every_section_of_a_narrowed_page_covers_the_range() {
    let (_root, r) = router();
    let wide = body(&r, "/stats?period=1y").await;
    let narrow = body(&r, "/stats?period=1y&from=2024-03-04&to=2024-03-10").await;
    assert_eq!(kpi_commits(&narrow), 3, "{narrow}");
    assert_eq!(
        kpi_commits(&narrow),
        bar_commits(&narrow),
        "the table and the per-day chart count the same commits"
    );
    assert!(kpi_commits(&wide) > kpi_commits(&narrow));
    let span = narrow
        .split("<p class=\"hint\">")
        .nth(1)
        .and_then(|s| s.split("</p>").next())
        .expect("span line");
    assert!(
        span.contains("2024-03-04") && span.contains("2024-03-10"),
        "{span}"
    );
    assert!(narrow.contains("Commits per day"), "{narrow}");
}
