//! The insights page for a revision too far from the indexed branch to serve from the index.
//! Reading every commit it lacks would hold that history in memory, so the page says why and
//! links to the same page on the indexed branch instead.

use askama::Template;

use crate::fmt;
use crate::pagehead::PageHead;
use crate::views::{self, Chrome};

#[derive(Template)]
#[template(path = "far.html")]
struct FarPage {
    c: Chrome,
    head: PageHead,
    why: String,
    /// The same tab without `?rev=`, so on the indexed branch.
    indexed: Option<String>,
}

/// Renders the page for tab `tab` of a revision past the cap.
pub(crate) fn render(c: Chrome, tab: &'static str, read: usize) -> askama::Result<String> {
    let indexed = fmt::with_rev("HEAD", || {
        let t = views::stats_tabs(tab);
        t.into_iter().find(|n| n.on).map(|n| n.href)
    });
    FarPage {
        c,
        head: crate::pagehead::insights_head(
            tab,
            crate::pagehead::Tagline::of(
                "Not served for this revision: it is too far from the indexed branch.",
            ),
            String::new(),
        ),
        why: format!(
            "Insights for this revision need a reindex: it has more than {} that the indexed \
             branch does not (counted {} before stopping), and loading them all would hold that \
             much history in memory.",
            gitterrace_index::count(gitterrace_index::feed::FRESH_CAP, "commit", "commits"),
            gitterrace_index::thousands(read)
        ),
        indexed,
    }
    .render()
}
