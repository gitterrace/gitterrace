//! The heads several pages share: insights, a file, the overview, refs and the working tree.

use crate::pagehead::{PageHead, Tab, Tagline};

/// The tagline of an insights tab: its finding (`crate::lead`), or a plain line when nothing
/// was counted.
pub(crate) fn lead_tagline(lead: &str) -> Tagline {
    if lead.is_empty() {
        Tagline::of("Nothing counted with these filters.")
    } else {
        Tagline::of(lead).lead()
    }
}

/// The page head of an insights tab: the title, the tagline, the five tabs and the toolbar.
pub(crate) fn insights_head(active: &str, tagline: Tagline, toolbar: String) -> PageHead {
    PageHead {
        tabs: crate::views::stats_tabs(active)
            .primary
            .into_iter()
            .map(|n| Tab {
                label: n.label.to_owned(),
                href: n.href,
                on: n.on,
            })
            .collect(),
        toolbar,
        ..PageHead::new("Insights", tagline)
    }
}
