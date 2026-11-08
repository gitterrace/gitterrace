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

/// Which view of one file a page shows, for its tab row.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileView {
    File,
    Blame,
    History,
}

/// The page head of a file's views: the path as the title, `tagline`, and File, Blame and
/// History as tabs (each only where its feature is on). `rev` is the revision in the path.
pub(crate) fn file_head(rev: &str, path: &str, on: FileView, tagline: Tagline) -> PageHead {
    use gitterrace_config::Feature;
    let (r, p) = (fmt::enc_seg(rev), fmt::enc_path(path));
    let mut tabs = vec![Tab {
        label: "File".to_owned(),
        href: fmt::href(&format!("/blob/{r}/{p}")),
        on: on == FileView::File,
    }];
    if crate::features::on(Feature::Blame) {
        tabs.push(Tab {
            label: "Blame".to_owned(),
            href: fmt::href(&format!("/blame/{r}/{p}")),
            on: on == FileView::Blame,
        });
    }
    if crate::features::on(Feature::History) {
        tabs.push(Tab {
            label: "History".to_owned(),
            href: fmt::href(&format!("/history/{p}")),
            on: on == FileView::History,
        });
    }
    PageHead {
        crumbs: crate::views::crumbs(rev, path, true),
        tabs,
        ..PageHead::new(path, tagline)
    }
}
