//! The insights page for a revision too far from the indexed branch to serve from the index.
//! Reading every commit it lacks would hold that history in memory, so the page says why and
//! links to the same page on the indexed branch instead.

use askama::Template;

use crate::fmt;
use crate::views::{self, Chrome, Tabs};

#[derive(Template)]
#[template(path = "far.html")]
struct FarPage {
    c: Chrome,
    tabs: Tabs,
    why: String,
    /// The same tab without `?rev=`, so on the indexed branch.
    indexed: Option<String>,
}

/// The sentence both the page and the API answer carry.
pub(crate) fn why(read: usize) -> String {
    format!(
        "Insights for this revision need a reindex: it has more than {} that the indexed branch \
         does not (counted {} before stopping), and loading them all would hold that much \
         history in memory.",
        gitterrace_index::count(gitterrace_index::feed::FRESH_CAP, "commit", "commits"),
        gitterrace_index::thousands(read)
    )
}

/// Renders the page for tab `tab` of a revision past the cap.
pub(crate) fn render(c: Chrome, tab: &'static str, read: usize) -> askama::Result<String> {
    let indexed = fmt::with_rev("HEAD", || {
        let t = views::stats_tabs(tab);
        t.primary.into_iter().find(|n| n.on).map(|n| n.href)
    });
    FarPage {
        c,
        tabs: views::stats_tabs(tab),
        why: why(read),
        indexed,
    }
    .render()
}
