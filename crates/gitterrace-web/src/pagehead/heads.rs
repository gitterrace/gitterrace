//! The heads several pages share: insights, a file, the overview, refs and the working tree.

use gitterrace_git::RefKind;

use crate::fmt;
use crate::pagehead::{Action, PageHead, Tab, Tagline};

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

/// The branches and tags page's head: how many of each kind of ref there are, from `total`,
/// and Compare from the newest tag (`compare`, `""` when the page is off).
pub(crate) fn refs_head(total: impl Fn(RefKind) -> usize, compare: String) -> PageHead {
    let said = format!(
        "{} \u{b7} {} \u{b7} {}",
        fmt::count(total(RefKind::Branch), "branch", "branches"),
        fmt::count(total(RefKind::Tag), "tag", "tags"),
        fmt::count(total(RefKind::Remote), "remote branch", "remote branches")
    );
    PageHead {
        actions: if compare.is_empty() {
            Vec::new()
        } else {
            vec![Action {
                label: "Compare",
                href: compare,
            }]
        },
        ..PageHead::new("Branches and tags", Tagline::of(&said))
    }
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

/// A changed file's head on the working tree: its path, linked back to the working tree page.
pub(crate) fn worktree_file_head(path: &str, tagline: Tagline) -> PageHead {
    PageHead {
        qualifier: "working tree".to_owned(),
        actions: vec![Action {
            label: "Working tree",
            href: fmt::href("/worktree"),
        }],
        ..PageHead::new(path, tagline)
    }
}

/// The overview's head: the repository's name, its branch and what was counted (`None` while
/// history is indexing).
pub(crate) fn overview_head(name: &str, branch: &str, counted: Option<&str>) -> PageHead {
    let said = format!(
        "On {branch} \u{b7} {}",
        counted.unwrap_or("indexing history")
    );
    PageHead::new(name, Tagline::of(&said))
}

/// The working tree's head: what differs from HEAD, read from disk.
pub(crate) fn worktree_head(files: usize, adds: u64, dels: u64) -> PageHead {
    let said = if files == 0 {
        "Clean: nothing differs from HEAD".to_owned()
    } else {
        format!(
            "{}, {} lines added and {} removed, read from disk and not committed",
            fmt::count(files, "changed file", "changed files"),
            fmt::thousands(adds),
            fmt::thousands(dels)
        )
    };
    PageHead {
        qualifier: "vs HEAD".to_owned(),
        ..PageHead::new("Working tree", Tagline::of(&said))
    }
}
