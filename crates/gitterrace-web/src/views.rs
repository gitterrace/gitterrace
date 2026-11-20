//! View models and the HTML fragments too hot to loop over in templates.

mod code;
mod diff;
mod look;

use gitterrace_git::{
    ChangeKind, CommitDetail, CommitSummary, FileDiff, RefInfo, TreeEntry, TreeEntryKind,
};
use gitterrace_types::ColorScheme;

pub(crate) use crate::commit_line::{CommitRow, Count, Lead};
use crate::fmt::{enc_path, enc_seg, esc_text, href};
#[cfg(test)]
pub(crate) use crate::views::code::code_html_first;
#[cfg(test)]
pub(crate) use crate::views::code::code_is_cached;
#[cfg(test)]
pub(crate) use crate::views::code::{CodeMemo, code_html, code_memo_has, code_memo_put};
pub(crate) use crate::views::code::{
    blame_first, blame_rows, clear_code_memo, code, code_first, code_tail_html,
};
pub(crate) use crate::views::diff::{
    DiffSource, MediaFacts, diff_fragment, diff_html_with, worktree_fragment, worktree_generated,
};
pub(crate) use crate::views::look::DiffLook;

/// The settings a rendered fragment depends on, read once per request from the config.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Render {
    /// Size of the highlighted-code cache, in bytes.
    pub(crate) cache_bytes: usize,
    /// Largest text highlighted, in bytes (`limits.highlight_kb`).
    pub(crate) highlight_bytes: usize,
    /// Lines a file page shows (`limits.blob_lines`).
    pub(crate) blob_lines: usize,
    /// Lines of one file's diff shown before it folds (`limits.diff_fold_lines`).
    pub(crate) fold_lines: usize,
}

impl Render {
    pub(crate) fn of(a: &crate::app::App) -> Self {
        let c = a.cfg();
        Self { cache_bytes: a.code_cache_bytes(), ..Self::from_config(&c, 0) }
    }

    /// The defaults, with a cache of `cache_bytes`.
    #[cfg(test)]
    pub(crate) fn test(cache_bytes: usize) -> Self {
        Self::from_config(&gitterrace_config::Config::default(), cache_bytes)
    }

    /// The defaults, folding at `fold_lines`.
    #[cfg(test)]
    pub(crate) fn fold(fold_lines: usize) -> Self {
        Self { fold_lines, ..Self::test(32 << 20) }
    }

    pub(crate) const fn from_config(c: &gitterrace_config::Config, cache_bytes: usize) -> Self {
        Self {
            cache_bytes,
            highlight_bytes: c.limits.highlight_kb.saturating_mul(1024),
            blob_lines: c.limits.blob_lines,
            fold_lines: c.limits.diff_fold_lines,
        }
    }
}

pub(crate) struct Nav {
    pub(crate) href: String,
    pub(crate) label: &'static str,
    /// Which page the entry opens, as the page names itself when it asks to be marked.
    pub(crate) key: &'static str,
    pub(crate) on: bool,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent page switches, each read by the shared chrome template"
)]
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
    /// Tint the line under the pointer (`display.has_line_hover`).
    pub(crate) has_line_hover: bool,
    /// How a diff looks (`display.diff_*`).
    pub(crate) diff: DiffLook,
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
    /// Which of the top bar's switchable tools the page shows.
    pub(crate) tools: Tools,
    /// The scripted features the config turned off, space separated, for the `gt-off` meta;
    /// empty (and no meta) when every one is on.
    pub(crate) off: String,
    /// Whether this server serves the settings page: the gear in the bar and the menu entry.
    pub(crate) settings: bool,
    /// Whose page this is: a server's own page (settings) keeps the brand, the switcher, the
    /// theme and the menu, and drops what needs a repository (branch, working tree).
    pub(crate) owner: Owner,
}

/// Whose page the bar is on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    /// A repository's page: the bar has its branch and working tree.
    Repo,
    /// The server's own page, which belongs to no repository.
    Server,
}

/// The top bar's tools a `[features]` switch can take away.
#[derive(Clone, Copy)]
pub(crate) struct Tools {
    /// `features.search_enabled`: the search button and the field it opens.
    pub(crate) search: bool,
    /// `features.picker_enabled`: the repository switcher.
    pub(crate) picker: bool,
    /// The server serves more than one repository, so the switcher shows from the first paint.
    pub(crate) many: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Crumb {
    pub(crate) name: String,
    pub(crate) href: String,
}

pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) kind: &'static str,
    pub(crate) size: String,
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

/// Consecutive commits that landed on one calendar day form one group. The log runs in the order
/// commits landed, so the committer date (in the committer's offset) follows it: a commit authored
/// earlier and landed later (rebased, cherry-picked) joins the day it landed instead of opening a
/// stray group. The row still shows when it was written.
pub(crate) fn day_groups(list: &[CommitSummary], now: i64) -> Vec<crate::routes::DayGroup> {
    let mut out: Vec<crate::routes::DayGroup> = Vec::new();
    for c in list {
        let label = crate::fmt::day_label(c.committer.time, c.committer.offset, now);
        match out.last_mut() {
            Some(g) if g.label == label => g.rows.push(commit_row(c, now)),
            _ => out.push(crate::routes::DayGroup { label, rows: vec![commit_row(c, now)] }),
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
    use gitterrace_config::Feature;
    let insights = crate::features::insights_home();
    let items = [
        Some(("/", "Code", "files")),
        Some(("/log/{rev}", "Commits", "log")),
        Some(("/refs", "Branches & Tags", "refs")),
        insights.map(|p| (p, "Insights", "stats")),
    ];
    Chrome {
        base: crate::fmt::base(),
        name: app.name.clone(),
        branch: app.repo.head_branch().unwrap_or_else(|| "HEAD".to_owned()),
        md: false,
        rev: rev.to_owned(),
        rev_enc: enc_seg(rev),
        commit: app.repo.resolve(rev).unwrap_or_default(),
        dirty: (!app.public() && crate::features::on(Feature::Worktree)).then(|| app.dirty()),
        // Links to pages computed from the selected ref carry the page's ref, whether it came
        // from the path (`/tree/dev`) or `?rev=`, so a tab change keeps the branch.
        nav: crate::fmt::with_rev(rev, || {
            items
                .into_iter()
                .flatten()
                .map(|(path, label, key)| Nav {
                    href: href(&path.replace("{rev}", &enc_seg(rev))),
                    label,
                    key,
                    on: key == active,
                })
                .collect()
        }),
        banner: if app.public() { None } else { banner(&app.repo.head_state(), &app.base) },
        actions_note: app.actions_note(),
        theme: match app.cfg().display.theme {
            ColorScheme::System => String::new(),
            t => t.as_str().to_owned(),
        },
        has_line_hover: app.cfg().display.has_line_hover,
        diff: DiffLook::of(&app.cfg().display),
        live: !app.public()
            && crate::features::on(Feature::Live)
            && app.cfg().watch.enabled
            && !crate::live::Mode::pinned_to(rev),
        live_how: "",
        live_on: "",
        version: app.version().to_string(),
        home: crate::fmt::with_rev(rev, || href("/")),
        // Read without waiting: the refresher computes it before ready, and a page that came
        // first (or while root walks of related repositories load the disk) must not block on
        // that walk. The permalink scripts skip an empty id.
        repo_id: app.repo.id_if_known().unwrap_or_default(),
        link_style: app.cfg().links.style.as_str(),
        repo_key: app.key.clone(),
        actions: app
            .actions
            .as_ref()
            .filter(|_| crate::features::on(Feature::Actions))
            .map(|l| (l.token().to_owned(), l.menu_json())),
        tools: Tools {
            search: crate::features::on(Feature::Search),
            picker: crate::features::on(Feature::Picker),
            many: app.env.served() > 1,
        },
        off: crate::features::scripted_off(),
        settings: app.settings_on() && crate::features::on(Feature::Settings),
        owner: Owner::Repo,
    }
}

impl Chrome {
    /// A server's own page, not a repository's.
    pub(crate) fn bare(&self) -> bool {
        self.owner == Owner::Server
    }

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
        Self { live_how: mode.how, live_on: mode.on, ..self }
    }

    /// Marks `key` as the open top nav entry, for a page that sits under another one.
    pub(crate) fn active(mut self, key: &str) -> Self {
        for n in &mut self.nav {
            n.on = n.key == key;
        }
        self
    }

    /// A page about a fixed commit or tag: nothing in the repository can change it.
    pub(crate) fn pinned(self) -> Self {
        Self { live: false, ..self }
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
    let at = h.commit.as_deref().map(|c| crate::fmt::sha_link_at(base, c)).unwrap_or_default();
    match (op, &h.branch) {
        (None, Some(_)) => None,
        (None, None) => Some(format!("HEAD is detached at {at}")),
        (Some(op), Some(b)) => Some(format!("{op} on {}", esc_text(b))),
        (Some(op), None) => Some(format!("{op}, HEAD detached at {at}")),
    }
}

/// Path, label and key of each insights tab, all in one row: is it alive, who works on it,
/// where the risk is, what it is made of and who knows it. Commit types and code frequency are
/// sections of Activity, patterns a section of Hotspots. Compare sits under Commits.
const STATS_TABS: [(&str, &str, &str); 5] = [
    ("/stats", "Activity", "a"),
    ("/stats/contributors", "Contributors", "co"),
    ("/stats/hotspots", "Hotspots", "ho"),
    ("/stats/languages", "Languages", "l"),
    ("/stats/ownership", "Ownership", "ow"),
];

/// The labels of the insights tabs, in tab order.
pub fn stats_tab_labels() -> Vec<&'static str> {
    STATS_TABS.iter().map(|(_, label, _)| *label).collect()
}

/// The insights tab row, every tab visible.
pub(crate) struct Tabs {
    pub(crate) primary: Vec<Nav>,
}

pub(crate) fn stats_tabs(active: &str) -> Tabs {
    // `insights.pages` orders the tabs.
    let all: Vec<Nav> = crate::features::pages()
        .iter()
        .filter_map(|p| crate::features::page_route(p))
        .filter_map(|r| STATS_TABS.iter().find(|t| t.0 == r).copied())
        .map(|(path, label, key)| Nav { href: href(path), label, key, on: key == active })
        .collect();
    Tabs { primary: all }
}

/// Avatar letters: the first letter of the first and of the last word of the name. One word
/// gives one letter, bracketed parts ("[bot]") and anything after an "@" are skipped, a CJK
/// name gives its first character, and nothing usable gives "?".
pub(crate) fn initials(name: &str) -> String {
    let mut plain = String::new();
    let mut depth = 0u32;
    for ch in name.chars() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => plain.push(ch),
            _ => {}
        }
    }
    let words: Vec<&str> = plain
        .split_whitespace()
        .map(|w| w.split('@').next().unwrap_or(""))
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .collect();
    let (Some(first), Some(last)) = (words.first(), words.last()) else {
        return "?".to_owned();
    };
    let head = letter(first);
    if is_cjk(&head) || words.len() == 1 {
        return head;
    }
    head + &letter(last)
}

/// The first letter of a word, uppercased, with the combining marks that follow it.
fn letter(word: &str) -> String {
    let mut chars = word.chars().skip_while(|c| !c.is_alphanumeric());
    let mut out: String = chars.next().into_iter().flat_map(char::to_uppercase).collect();
    out.extend(chars.take_while(|&c| is_combining(c)));
    out
}

fn is_combining(c: char) -> bool {
    matches!(
        u32::from(c),
        0x300..=0x36F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
    )
}

fn is_cjk(s: &str) -> bool {
    s.chars().next().is_some_and(|c| {
        matches!(
            u32::from(c),
            0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF
        )
    })
}

/// Stable avatar hue per name.
pub(crate) fn hue(name: &str) -> u32 {
    name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))) % 360
}

/// Link to the commit list of one author.
pub(crate) fn author_href(email: &str) -> String {
    log_href(&crate::filter::Filter::default(), crate::filter::Dim::Author, email)
}

/// The log of the selected revision with `d` set to `value`: every link into a filtered log.
pub(crate) fn log_href(f: &crate::filter::Filter, d: crate::filter::Dim, value: &str) -> String {
    f.link(&format!("/log/{}", crate::fmt::selected_seg()), d, value)
}

pub(crate) fn commit_row(c: &CommitSummary, now: i64) -> CommitRow {
    CommitRow::of(c, now)
}

/// Breadcrumbs for `path` under `rev`; `kind` is `tree` for every part but the last of a blob.
pub(crate) fn crumbs(rev: &str, path: &str, last_is_file: bool) -> Vec<Crumb> {
    let r = enc_seg(rev);
    let mut out = vec![Crumb { name: rev.to_owned(), href: href(&format!("/tree/{r}")) }];
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let mut acc = String::new();
    for (i, p) in parts.iter().enumerate() {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(p);
        let kind = if last_is_file && i + 1 == parts.len() { "blob" } else { "tree" };
        out.push(Crumb {
            name: (*p).to_owned(),
            href: href(&format!("/{kind}/{r}/{}", enc_path(&acc))),
        });
    }
    out
}

pub(crate) fn entries(rev: &str, path: &str, list: &[TreeEntry]) -> Vec<Entry> {
    let base = if path.is_empty() { String::new() } else { format!("{}/", path.trim_matches('/')) };
    list.iter()
        .map(|e| {
            let full = enc_path(&format!("{base}{}", e.name));
            let r = enc_seg(rev);
            let (href, kind) = match e.kind {
                TreeEntryKind::Dir => (href(&format!("/tree/{r}/{full}")), "dir"),
                TreeEntryKind::File => (
                    href(&format!("/blob/{r}/{full}")),
                    if crate::media::is_image(&e.name) { "img" } else { "file" },
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

/// The rows of the newest `limit` refs of `kind`, each led by its name: the one commit row,
/// newest first. `refs` comes newest first, so only the rows shown read their commit.
pub(crate) fn ref_rows(
    app: &crate::app::App,
    refs: &[RefInfo],
    kind: gitterrace_git::RefKind,
    now: i64,
    limit: usize,
) -> crate::error::Result<Vec<CommitRow>> {
    let picked: Vec<&RefInfo> = refs.iter().filter(|r| r.kind == kind).take(limit).collect();
    let ids: Vec<String> = picked.iter().map(|r| r.target.clone()).collect();
    let page = if kind == gitterrace_git::RefKind::Tag { "tag" } else { "log" };
    Ok(app
        .repo
        .summaries(&ids)?
        .iter()
        .zip(picked)
        .map(|(c, r)| {
            // An annotated tag's own subject says more about it than its commit's does.
            let note = r.annotation.as_ref().map(|n| n.subject.clone()).unwrap_or_default();
            CommitRow::of(c, now).note(note).led_by(Lead {
                name: r.name.clone(),
                href: href(&format!("/{page}/{}", enc_seg(&r.name))),
                badges: Vec::new(),
            })
        })
        .collect())
}

pub(crate) fn file_label(f: &FileDiff) -> String {
    match &f.old_path {
        Some(old) if f.kind == ChangeKind::Renamed => format!("{old} -> {}", f.path),
        _ => f.path.clone(),
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

#[cfg(test)]
mod tests;
