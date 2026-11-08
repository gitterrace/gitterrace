//! The heads several pages share: insights, a file, the overview, refs and the working tree.

use gitterrace_git::RefKind;

use crate::fmt;
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

/// The branches and tags page's head: how many of each kind of ref there are, from `total`.
pub(crate) fn refs_head(total: impl Fn(RefKind) -> usize) -> PageHead {
    let said = format!(
        "{} \u{b7} {} \u{b7} {}",
        fmt::count(total(RefKind::Branch), "branch", "branches"),
        fmt::count(total(RefKind::Tag), "tag", "tags"),
        fmt::count(total(RefKind::Remote), "remote branch", "remote branches")
    );
    PageHead::new("Branches and tags", Tagline::of(&said))
}

/// The head of an insights tab while the history index is built.
pub(crate) fn waiting_head(tab: &str) -> PageHead {
    insights_head(
        tab,
        Tagline::of("Waiting for the history index."),
        String::new(),
    )
}
