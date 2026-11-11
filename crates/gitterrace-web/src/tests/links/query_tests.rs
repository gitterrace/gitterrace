//! A link's query string must change the page it lands on: a parameter the handler (or its
//! memo key) ignores is a link that silently does nothing.

use std::collections::BTreeSet;

use crate::tests::get;

/// The pages the crawl starts from: every insights page, the log and a file page.
const SEEDS: [&str; 8] = [
    "/stats",
    "/stats/hotspots",
    "/stats/ownership",
    "/stats/contributors",
    "/stats/languages",
    "/log/HEAD",
    "/history/HEAD/old.rs",
    "/blob/HEAD/old.rs",
];

/// Same-site `href`s with a query string, `&amp;` decoded, the `#fragment` dropped.
fn query_links(body: &str) -> BTreeSet<String> {
    body.split("href=\"")
        .skip(1)
        .filter_map(|a| a.split('"').next())
        .map(|h| h.replace("&amp;", "&"))
        .map(|h| h.split('#').next().unwrap_or_default().to_owned())
        .filter(|h| h.starts_with('/') && !h.starts_with("//") && h.contains('?'))
        .filter(|h| !h.starts_with("/static/") && !h.starts_with("/api/"))
        .collect()
}

/// The page without what varies between two renders of the same thing.
fn content(body: &str) -> String {
    let main = body.split("<main").nth(1).unwrap_or(body);
    main.split("</main>").next().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn every_query_link_changes_the_page_it_lands_on() {
    let dir = tempfile::tempdir().expect("tmp");
    let app = super::period_tests::app(dir.path());
    let mut links = BTreeSet::new();
    for seed in SEEDS {
        let (s, body) = get(&app, seed).await;
        assert_eq!(s, 200, "{seed}");
        links.extend(query_links(&body));
    }
    let mut dead = Vec::new();
    for link in &links {
        let (path, _) = link.split_once('?').unwrap_or((link, ""));
        let (s, with) = get(&app, link).await;
        if s != 200 {
            continue;
        }
        let (_, without) = get(&app, path).await;
        if content(&with) == content(&without) {
            dead.push(link.clone());
        }
    }
    assert!(
        dead.is_empty(),
        "links whose query does nothing:\n{}",
        dead.join("\n")
    );
}
