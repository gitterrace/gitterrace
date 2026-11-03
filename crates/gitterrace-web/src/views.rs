//! View models and the HTML fragments too hot to loop over in templates.

mod code;
mod diff;

use gitterrace_git::{
    ChangeKind, CommitDetail, CommitSummary, FileDiff, RefInfo, TreeEntry, TreeEntryKind,
};

use crate::fmt::{ago, date, enc_path, enc_seg, esc_text, href, sha_link};
#[cfg(test)]
pub(crate) use crate::views::code::{CodeMemo, code_html, code_memo_has, code_memo_put};
pub(crate) use crate::views::code::{blame_html, code_html_first, code_tail_html};
pub(crate) use crate::views::diff::{
    DiffSource, diff_fragment, diff_html, diff_html_with, worktree_fragment,
};

const MAX_BLOB_LINES: usize = 20_000;

pub(crate) struct Nav {
    pub(crate) href: String,
    pub(crate) label: &'static str,
    pub(crate) on: bool,
}

/// The Branches & tags, Worktrees and Stashes tab bar, the same on all three pages: every tab
/// shows its count, 0 included, so a count never appears on one page and vanishes on the next.
pub(crate) struct RefTab {
    pub(crate) href: String,
    pub(crate) label: &'static str,
    pub(crate) count: usize,
    pub(crate) on: bool,
}

/// The counts come from the same repository reads on every page.
pub(crate) fn ref_tabs(app: &crate::app::App, on: &str) -> crate::error::Result<Vec<RefTab>> {
    // Branches and tags together, as the tab is labelled; remotes have their own section.
    let local = app
        .repo
        .refs()?
        .iter()
        .filter(|r| r.kind != gitterrace_git::RefKind::Remote)
        .count();
    let tab = |key: &str, label, count| RefTab {
        href: crate::fmt::href(&format!("/{key}")),
        label,
        count,
        on: key == on,
    };
    let mut tabs = vec![tab("refs", "Branches & tags", local)];
    // Worktrees and stashes are this machine's state, not history.
    if !app.public() {
        tabs.push(tab(
            "worktrees",
            "Worktrees",
            app.repo.linked_worktrees()?.len(),
        ));
        tabs.push(tab("stashes", "Stashes", app.repo.stashes()?.len()));
    }
    Ok(tabs)
}

pub(crate) struct Chrome {
    /// Prefix for absolute links; empty for a single repository.
    pub(crate) base: String,
    pub(crate) name: String,
    pub(crate) branch: String,
    /// Page contains rendered markdown, so the markdown script is loaded.
    pub(crate) md: bool,
    /// Revision as written in the URL, used by the finder and the ref switcher.
    pub(crate) rev: String,
    pub(crate) rev_enc: String,
    /// Full commit id of `rev`, for permalinks.
    pub(crate) commit: String,
    /// Worktree differs from HEAD: show the Working tree chip. `None` on a public server, where
    /// the chip is not even rendered hidden, so no page links a local route.
    pub(crate) dirty: Option<bool>,
    pub(crate) nav: Vec<Nav>,
    /// HEAD detached or an operation half done, worded for the banner; `None` on a plain branch.
    pub(crate) banner: Option<String>,
    /// Why `actions.toml` is not in force, shown under the bar from the first paint.
    pub(crate) actions_note: Option<String>,
    /// The repository's overview, keeping the selected ref.
    pub(crate) home: String,
    /// Repo id for permalinks (`/go/<id>/...`); empty for a repo without commits.
    pub(crate) repo_id: String,
    /// Form Copy permalink and Send to hand out (`links.style`): `gitterrace` or `go`.
    pub(crate) link_style: &'static str,
    /// Name run requests use for this repo.
    pub(crate) repo_key: String,
    /// Color theme before the visitor picks one (`display.theme`), empty for the system's.
    pub(crate) theme: String,
    /// Open a live-update stream: off on pages pinned to a commit or tag.
    pub(crate) live: bool,
    /// What the page does with a change: `refresh`, `notice` or nothing but the chrome.
    pub(crate) live_how: &'static str,
    /// Kinds of change the page shows, space separated.
    pub(crate) live_on: &'static str,
    /// Fingerprint the page was rendered at, so a reconnect can tell it missed a change.
    pub(crate) version: String,
    /// Per-process action token and the menu as JSON; `None` when actions are off, so pages
    /// carry neither.
    pub(crate) actions: Option<(String, String)>,
}

pub(crate) struct Crumb {
    pub(crate) name: String,
    pub(crate) href: String,
}

pub(crate) struct CommitRow {
    /// The commit id as its link, finished HTML from `fmt::sha_link`.
    pub(crate) sha: String,
    pub(crate) initial: String,
    pub(crate) hue: u32,
    pub(crate) href: String,
    pub(crate) subject: String,
    pub(crate) author: String,
    /// Commit list narrowed to this author, matched by (mailmapped) email.
    pub(crate) author_href: String,
    pub(crate) when: String,
    pub(crate) date: String,
}

pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) kind: &'static str,
    pub(crate) size: String,
}

pub(crate) struct RefRow {
    pub(crate) name: String,
    pub(crate) log: String,
    pub(crate) commit: String,
    pub(crate) when: String,
}

pub(crate) struct FileStatRow {
    pub(crate) anchor: String,
    pub(crate) label: String,
    pub(crate) additions: String,
    pub(crate) deletions: String,
}

pub(crate) struct Row {
    pub(crate) label: String,
    pub(crate) href: String,
    pub(crate) commits: String,
    pub(crate) additions: String,
    pub(crate) deletions: String,
    pub(crate) pct: u32,
}

/// Consecutive commits sharing a calendar day (in the author's offset) form one group.
pub(crate) fn day_groups(list: &[CommitSummary], now: i64) -> Vec<crate::routes::DayGroup> {
    let mut out: Vec<crate::routes::DayGroup> = Vec::new();
    for c in list {
        let label = crate::fmt::day_label(c.author.time, c.author.offset, now);
        match out.last_mut() {
            Some(g) if g.label == label => g.rows.push(commit_row(c, now)),
            _ => out.push(crate::routes::DayGroup {
                label,
                rows: vec![commit_row(c, now)],
            }),
        }
    }
    out
}

pub(crate) fn chrome(app: &crate::app::App, active: &str) -> Chrome {
    chrome_at(app, active, &crate::fmt::selected())
}

#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "`{rev}` and `{dir}` are our own placeholders, not format arguments"
)]
pub(crate) fn chrome_at(app: &crate::app::App, active: &str, rev: &str) -> Chrome {
    let items = [
        ("/", "Code", "files"),
        ("/log/{rev}", "Commits", "log"),
        ("/refs", "Branches & Tags", "refs"),
        ("/stats", "Insights", "stats"),
    ];
    Chrome {
        base: crate::fmt::base(),
        name: app.name.clone(),
        branch: app.repo.head_branch().unwrap_or_else(|| "HEAD".to_owned()),
        md: false,
        rev: rev.to_owned(),
        rev_enc: enc_seg(rev),
        commit: app.repo.resolve(rev).unwrap_or_default(),
        dirty: (!app.public()).then(|| app.dirty()),
        nav: items
            .into_iter()
            .map(|(path, label, key)| Nav {
                href: href(&path.replace("{rev}", &enc_seg(rev))),
                label,
                on: key == active,
            })
            .collect(),
        banner: if app.public() {
            None
        } else {
            banner(&app.repo.head_state(), &app.base)
        },
        actions_note: app.actions_note(),
        theme: match app.cfg().display.theme.as_str() {
            "system" => String::new(),
            t => t.to_owned(),
        },
        live: !app.public() && app.cfg().watch.enabled && !crate::live::Mode::pinned_to(rev),
        live_how: "",
        live_on: "",
        version: app.version().to_string(),
        home: href("/"),
        repo_id: app.repo.id().unwrap_or_default(),
        link_style: gitterrace_actions::LinkStyle::from_config(&app.cfg().links.style).as_str(),
        repo_key: app.key.clone(),
        actions: app
            .actions
            .as_ref()
            .map(|l| (l.token().to_owned(), l.menu_json())),
    }
}

impl Chrome {
    /// What the ref switcher button shows: the branch for HEAD, the abbreviated id when the page
    /// is pinned to a full commit id, otherwise the ref as written.
    pub(crate) fn switcher_label(&self) -> &str {
        if self.rev == "HEAD" {
            &self.branch
        } else if self.rev == self.commit {
            gitterrace_git::short_id(&self.rev)
        } else {
            &self.rev
        }
    }

    /// How this page reacts to changes in the repository.
    pub(crate) fn live(self, mode: crate::live::Mode) -> Self {
        Self {
            live_how: mode.how,
            live_on: mode.on,
            ..self
        }
    }

    /// A page about a fixed commit or tag: nothing in the repository can change it.
    pub(crate) fn pinned(self) -> Self {
        Self {
            live: false,
            ..self
        }
    }
}

/// What the banner says about HEAD, as HTML (the commit is a link). Nothing on a branch with no
/// operation in progress.
pub(crate) fn banner(h: &gitterrace_git::HeadState, base: &str) -> Option<String> {
    use gitterrace_git::InProgress as P;
    let op = h.in_progress.map(|p| match p {
        P::Rebase => "Rebase in progress",
        P::Merge => "Merge in progress",
        P::CherryPick => "Cherry-pick in progress",
        P::Revert => "Revert in progress",
        P::Bisect => "Bisect in progress",
        P::ApplyMailbox => "Applying patches (git am) in progress",
    });
    let at = h
        .commit
        .as_deref()
        .map(|c| crate::fmt::sha_link_at(base, c))
        .unwrap_or_default();
    match (op, &h.branch) {
        (None, Some(_)) => None,
        (None, None) => Some(format!("HEAD is detached at {at}")),
        (Some(op), Some(b)) => Some(format!("{op} on {}", esc_text(b))),
        (Some(op), None) => Some(format!("{op}, HEAD detached at {at}")),
    }
}

/// Path, label and key of each insights tab. The first [`PRIMARY_TABS`] are what a first visit
/// needs (is it alive, who works on it, where the risk is, what it is made of); the rest sit
/// behind one "More" disclosure.
const STATS_TABS: [(&str, &str, &str); 9] = [
    ("/stats", "Activity", "a"),
    ("/stats/contributors", "Contributors", "co"),
    ("/stats/hotspots", "Hotspots", "ho"),
    ("/stats/languages", "Languages", "l"),
    ("/stats/ownership", "Ownership", "ow"),
    ("/stats/types", "Commit types", "t"),
    ("/stats/frequency", "Code frequency", "cf"),
    ("/stats/patterns", "Patterns", "pa"),
    ("/compare", "Compare", "cm"),
];
const PRIMARY_TABS: usize = 4;

/// The labels of the insights tabs, in tab order.
pub fn stats_tab_labels() -> Vec<&'static str> {
    STATS_TABS.iter().map(|(_, label, _)| *label).collect()
}

/// The insights tab row: the primary tabs, then the rest under "More", open when one of them
/// is the current page so the active tab is always visible.
pub(crate) struct Tabs {
    pub(crate) primary: Vec<Nav>,
    pub(crate) more: Vec<Nav>,
    pub(crate) more_on: bool,
}

pub(crate) fn stats_tabs(active: &str) -> Tabs {
    let mut all: Vec<Nav> = STATS_TABS
        .into_iter()
        .map(|(path, label, key)| Nav {
            href: href(path),
            label,
            on: key == active,
        })
        .collect();
    let more = all.split_off(PRIMARY_TABS);
    Tabs {
        more_on: more.iter().any(|n| n.on),
        primary: all,
        more,
    }
}

/// First letter of a name for the avatar disc.
pub(crate) fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(|| "?".to_owned(), |ch| ch.to_uppercase().collect())
}

/// Stable avatar hue per name.
pub(crate) fn hue(name: &str) -> u32 {
    name.bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)))
        % 360
}

/// Link to the commit list of one author.
pub(crate) fn author_href(email: &str) -> String {
    href(&format!(
        "/log/{}?author={}",
        crate::fmt::selected_seg(),
        enc_seg(email)
    ))
}

pub(crate) fn commit_row(c: &CommitSummary, now: i64) -> CommitRow {
    CommitRow {
        sha: sha_link(&c.id),
        initial: initial(&c.author.name),
        hue: hue(&c.author.name),
        href: href(&format!("/commit/{}", c.id)),
        subject: c.subject.clone(),
        author: c.author.name.clone(),
        author_href: author_href(&c.author.email),
        when: ago(c.author.time, now),
        date: date(c.author.time, c.author.offset),
    }
}

/// Breadcrumbs for `path` under `rev`; `kind` is `tree` for every part but the last of a blob.
pub(crate) fn crumbs(rev: &str, path: &str, last_is_file: bool) -> Vec<Crumb> {
    let r = enc_seg(rev);
    let mut out = vec![Crumb {
        name: rev.to_owned(),
        href: href(&format!("/tree/{r}")),
    }];
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let mut acc = String::new();
    for (i, p) in parts.iter().enumerate() {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(p);
        let kind = if last_is_file && i + 1 == parts.len() {
            "blob"
        } else {
            "tree"
        };
        out.push(Crumb {
            name: (*p).to_owned(),
            href: href(&format!("/{kind}/{r}/{}", enc_path(&acc))),
        });
    }
    out
}

pub(crate) fn entries(rev: &str, path: &str, list: &[TreeEntry]) -> Vec<Entry> {
    let base = if path.is_empty() {
        String::new()
    } else {
        format!("{}/", path.trim_matches('/'))
    };
    list.iter()
        .map(|e| {
            let full = enc_path(&format!("{base}{}", e.name));
            let r = enc_seg(rev);
            let (href, kind) = match e.kind {
                TreeEntryKind::Dir => (href(&format!("/tree/{r}/{full}")), "dir"),
                TreeEntryKind::File => (
                    href(&format!("/blob/{r}/{full}")),
                    if crate::media::is_image(&e.name) {
                        "img"
                    } else {
                        "file"
                    },
                ),
                TreeEntryKind::Symlink => (href(&format!("/blob/{r}/{full}")), "link"),
                TreeEntryKind::Submodule => (String::new(), "module"),
            };
            Entry {
                name: e.name.clone(),
                href,
                kind,
                size: e.size.map(crate::fmt::size).unwrap_or_default(),
            }
        })
        .collect()
}

pub(crate) fn ref_rows(refs: &[RefInfo], kind: gitterrace_git::RefKind, now: i64) -> Vec<RefRow> {
    refs.iter()
        .filter(|r| r.kind == kind)
        .map(|r| RefRow {
            name: r.name.clone(),
            log: href(&format!("/log/{}", enc_seg(&r.name))),
            commit: sha_link(&r.target),
            when: ago(r.time, now),
        })
        .collect()
}

pub(crate) fn file_label(f: &FileDiff) -> String {
    match &f.old_path {
        Some(old) if f.kind == ChangeKind::Renamed => format!("{old} -> {}", f.path),
        _ => f.path.clone(),
    }
}

pub(super) const fn kind_name(k: ChangeKind) -> &'static str {
    match k {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
    }
}

pub(crate) fn file_stats(d: &CommitDetail) -> Vec<FileStatRow> {
    d.files
        .iter()
        .enumerate()
        .map(|(i, f)| FileStatRow {
            anchor: format!("f{i}"),
            label: file_label(f),
            additions: crate::fmt::thousands(f.additions),
            deletions: crate::fmt::thousands(f.deletions),
        })
        .collect()
}

/// Link to the history page of one file.
pub(crate) fn file_link(path: &str) -> String {
    crate::fmt::href(&format!("/history/{}", crate::fmt::enc_path(path)))
}

#[cfg(test)]
mod tests;
