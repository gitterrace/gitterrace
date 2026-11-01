//! View models and the HTML fragments too hot to loop over in templates.

use std::fmt::Write as _;

use gitterrace_git::{
    BlameLine, ChangeKind, CommitDetail, CommitSummary, FileDiff, LineKind, RefInfo, TreeEntry,
    TreeEntryKind,
};

use crate::fmt::{ago, date, enc_path, enc_seg, esc, esc_text, href, sha_link};

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
    Ok(vec![
        tab("refs", "Branches & tags", local),
        tab("worktrees", "Worktrees", app.repo.linked_worktrees()?.len()),
        tab("stashes", "Stashes", app.repo.stashes()?.len()),
    ])
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
    /// Worktree differs from HEAD: show the Working tree chip.
    pub(crate) dirty: bool,
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
        dirty: app.dirty(),
        nav: items
            .into_iter()
            .map(|(path, label, key)| Nav {
                href: href(&path.replace("{rev}", &enc_seg(rev))),
                label,
                on: key == active,
            })
            .collect(),
        banner: banner(&app.repo.head_state(), &app.base),
        actions_note: app.actions_note(),
        theme: match app.cfg().display.theme.as_str() {
            "system" => String::new(),
            t => t.to_owned(),
        },
        live: app.cfg().watch.enabled && !crate::live::Mode::pinned_to(rev),
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

const fn kind_name(k: ChangeKind) -> &'static str {
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

fn num(n: Option<u32>) -> String {
    n.map_or_else(String::new, |n| n.to_string())
}

pub(crate) fn diff_html(
    fold_lines: usize,
    rev: &str,
    parent: Option<&str>,
    files: &[FileDiff],
) -> String {
    diff_html_with(fold_lines, Some(rev), parent, files)
}

/// Once the files before it changed this many lines in all, later files are folded too.
const TOTAL_DIFF_BUDGET: usize = 2_000;
/// The same budgets in rendered bytes, since a few huge lines cost more than many short ones.
const FILE_DIFF_BYTES: usize = 40_000;
const TOTAL_DIFF_BYTES: usize = 250_000;
/// Lines per `content-visibility` chunk (the stylesheet sizes a chunk as this many rows).
const CHUNK_LINES: usize = 100;

/// The hunks of one file as line blocks. Also the body of the fragment a folded file loads.
fn diff_body_html(f: &FileDiff) -> String {
    let mut out = String::from("<div class=\"diffwrap\"><div class=\"diff\">");
    let mut in_chunk = 0usize;
    let total: usize = f
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| l.text.len())
        .sum();
    let plain = total > crate::highlight::HIGHLIGHT_LIMIT;
    let first = |k: LineKind| {
        f.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .find(|l| l.kind == k)
            .map_or("", |l| l.text.as_str())
    };
    let old_path = f.old_path.as_deref().unwrap_or(&f.path);
    let mut new_hl = (!plain)
        .then(|| crate::highlight::for_file(&f.path, first(LineKind::Added)))
        .flatten();
    let mut old_hl = (!plain)
        .then(|| crate::highlight::for_file(old_path, first(LineKind::Removed)))
        .flatten();
    for h in &f.hunks {
        if in_chunk == 0 {
            out.push_str("<div class=\"ch\">");
        }
        let _ = write!(out, "<div class=\"hh\">{}</div>", esc(&h.header));
        in_chunk += 1;
        for l in &h.lines {
            if in_chunk == 0 {
                out.push_str("<div class=\"ch\">");
            }
            let cls = match l.kind {
                LineKind::Added => "add",
                LineKind::Removed => "del",
                LineKind::Context => "ctx",
            };
            let hl = if l.kind == LineKind::Removed {
                old_hl.as_mut()
            } else {
                new_hl.as_mut()
            };
            let body = hl.map_or_else(|| esc_text(&l.text), |h| h.line(&l.text));
            let _ = write!(
                out,
                "<div class=\"{cls}\" data-o=\"{}\" data-n=\"{}\">{body}</div>",
                num(l.old_no),
                num(l.new_no)
            );
            in_chunk += 1;
            if in_chunk >= CHUNK_LINES {
                out.push_str("</div>");
                in_chunk = 0;
            }
        }
    }
    if in_chunk != 0 {
        out.push_str("</div>");
    }
    out.push_str("</div></div>");
    out
}

/// `rev` of `None` renders file names without links (the worktree has no blob URLs). A file
/// whose changed lines pass `fold_lines` is folded behind a "Load diff" button.
pub(crate) fn diff_html_with(
    fold_lines: usize,
    rev: Option<&str>,
    parent: Option<&str>,
    files: &[FileDiff],
) -> String {
    let mut out = String::new();
    let mut budget_used = 0usize;
    let mut bytes_used = 0usize;
    for (i, f) in files.iter().enumerate() {
        let _ = write!(
            out,
            "<section class=\"file\" id=\"f{i}\" data-path=\"{p}\"><header><span class=\"badge {k}\">{k}</span> ",
            k = kind_name(f.kind),
            p = esc(&f.path)
        );
        if f.kind == ChangeKind::Deleted {
            out.push_str(&esc(&f.path));
        } else if let Some(rev) = rev {
            let _ = write!(
                out,
                "<a href=\"{}\">{}</a>",
                crate::fmt::href(&format!("/blob/{}/{}", enc_seg(rev), enc_path(&f.path))),
                esc(&file_label(f))
            );
        } else {
            out.push_str(&esc(&file_label(f)));
        }
        let _ = write!(
            out,
            "<span class=\"counts\"><b class=\"add\">+{}</b> <b class=\"del\">-{}</b></span></header>",
            crate::fmt::thousands(f.additions),
            crate::fmt::thousands(f.deletions)
        );
        if f.binary && crate::media::is_image(&f.path) && rev.is_some() {
            let old_path = f.old_path.as_deref().unwrap_or(&f.path);
            let old = (f.kind != ChangeKind::Added)
                .then_some(parent)
                .flatten()
                .map(|p| (p, old_path));
            let new = (f.kind != ChangeKind::Deleted)
                .then_some(rev)
                .flatten()
                .map(|r| (r, f.path.as_str()));
            out.push_str(&crate::media::diff_html(old, new));
        } else if f.binary {
            out.push_str("<p class=\"muted pad\">Binary file not shown.</p>");
        } else if f.hunks.is_empty() {
            out.push_str("<p class=\"muted pad\">No textual changes.</p>");
        } else {
            let changed = (f.additions + f.deletions) as usize;
            let bytes: usize = f
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .map(|l| l.text.len() + 48)
                .sum();
            let over = rev.is_some()
                && (changed > fold_lines
                    || bytes > FILE_DIFF_BYTES
                    || budget_used + changed > TOTAL_DIFF_BUDGET
                    || bytes_used + bytes > TOTAL_DIFF_BYTES);
            budget_used += changed;
            bytes_used += bytes;
            if let (true, Some(rev)) = (over, rev) {
                let _ = write!(
                    out,
                    "<p class=\"muted pad\"><button type=\"button\" class=\"linkbtn\" data-load-diff=\"{}/diff/{}/{i}\">Load diff ({} changed)</button></p>",
                    crate::fmt::base(),
                    enc_seg(rev),
                    crate::fmt::count(changed, "line", "lines")
                );
            } else {
                out.push_str(&diff_body_html(f));
            }
        }
        out.push_str("</section>");
    }
    out
}

/// `GET /diff/{rev}/{index}`: the body of one folded file of a commit, as a fragment.
pub(crate) async fn diff_fragment(
    axum::extract::State(app): axum::extract::State<std::sync::Arc<crate::app::App>>,
    axum::extract::Path((rev, index)): axum::extract::Path<(String, usize)>,
) -> crate::error::Result<axum::response::Html<String>> {
    let html = crate::app::blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        Ok(a.repo.commit_file(&id, index)?.as_ref().map(diff_body_html))
    })
    .await?;
    html.map(axum::response::Html)
        .ok_or_else(|| gitterrace_git::Error::PathNotFound(format!("file {index}")).into())
}

/// Chunks highlighted before first paint of a big file; the rest is plain until `code_tail_html`.
const EAGER_CHUNKS: usize = 2;
/// Files up to this size are highlighted whole (about 20 ms): the fragment round trip is not worth it.
const EAGER_BYTES: usize = 32 * 1024;

type Memo = Option<(usize, std::collections::HashMap<u64, (String, bool)>)>;
static CODE_CACHE: std::sync::Mutex<Memo> = std::sync::Mutex::new(None);

fn code_key(path: &str, text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path, text).hash(&mut hasher);
    hasher.finish()
}

/// Whether the highlighted lines of this file are in the cache.
#[cfg(test)]
pub(crate) fn code_is_cached(path: &str, text: &str) -> bool {
    code_cached(code_key(path, text)).is_some()
}

fn code_cached(key: u64) -> Option<(String, bool)> {
    let g = CODE_CACHE.lock().ok()?;
    g.as_ref()?.1.get(&key).cloned()
}

/// Returns the line blocks and whether it was cut at the line cap.
/// One div per line: the number is a CSS counter, so it neither selects nor copies.
/// Highlighting costs ~0.7 ms/KB, so a blob is rendered once per content.
pub(crate) fn code_html(cache_bytes: usize, path: &str, text: &str) -> (String, bool) {
    let key = code_key(path, text);
    if let Some(hit) = code_cached(key) {
        return hit;
    }
    let built = code_html_uncached(path, text, usize::MAX);
    if let Ok(mut g) = CODE_CACHE.lock() {
        let (size, map) = g.get_or_insert_with(Default::default);
        if *size + built.0.len() > cache_bytes {
            map.clear();
            *size = 0;
        }
        *size += built.0.len();
        map.insert(key, built.clone());
    }
    built
}

/// Like `code_html`, but a big uncached file highlights only its first chunks and marks the
/// block `data-hl` so the page fetches the rest from `code_tail_html` after first paint.
pub(crate) fn code_html_first(cache_bytes: usize, path: &str, text: &str) -> (String, bool) {
    if text.len() <= EAGER_BYTES || code_cached(code_key(path, text)).is_some() {
        return code_html(cache_bytes, path, text);
    }
    code_html_uncached(path, text, EAGER_CHUNKS * CHUNK_LINES)
}

/// The chunks `code_html_first` left plain, highlighted.
pub(crate) fn code_tail_html(cache_bytes: usize, path: &str, text: &str) -> String {
    let (full, _) = code_html(cache_bytes, path, text);
    let marker = "<div class=\"ch\"";
    let from = full
        .match_indices(marker)
        .nth(EAGER_CHUNKS)
        .map_or(full.len(), |m| m.0);
    full[from..full.len() - "</div>".len()].to_owned()
}

fn code_html_uncached(path: &str, text: &str, lit_lines: usize) -> (String, bool) {
    let mut hl = (text.len() <= crate::highlight::HIGHLIGHT_LIMIT)
        .then(|| crate::highlight::for_file(path, text.lines().next().unwrap_or("")))
        .flatten();
    let lazy = hl.is_some() && lit_lines != usize::MAX;
    let mut out = String::with_capacity(text.len() + text.len() / 8 + 64);
    out.push_str(if lazy {
        "<div class=\"code src\" data-hl>"
    } else {
        "<div class=\"code src\">"
    });
    let mut cut = false;
    let mut open = false;
    for (i, line) in text.lines().enumerate() {
        if i >= MAX_BLOB_LINES {
            cut = true;
            break;
        }
        if i == lit_lines {
            hl = None;
        }
        if i % CHUNK_LINES == 0 {
            if open {
                out.push_str("</div>");
            }
            out.push_str("<div class=\"ch\">");
            open = true;
        }
        // `counter-reset` on the chunk would open a new counter scope there, which resets
        // the number to 1 for each chunk. `counter-set` on the first line keeps a single counter.
        let set = if i > 0 && i % CHUNK_LINES == 0 {
            format!(" style=\"counter-set:n {i}\"")
        } else {
            String::new()
        };
        let _ = write!(
            out,
            "<div id=\"L{}\" class=\"l\"{set}>{}</div>",
            i + 1,
            hl.as_mut().map_or_else(|| esc_text(line), |h| h.line(line))
        );
    }
    if open {
        out.push_str("</div>");
    }
    out.push_str("</div>");
    (out, cut)
}

pub(crate) fn blame_html(path: &str, lines: &[BlameLine], now: i64) -> String {
    let total: usize = lines.iter().map(|l| l.text.len()).sum();
    let mut hl = (total <= crate::highlight::HIGHLIGHT_LIMIT)
        .then(|| crate::highlight::for_file(path, lines.first().map_or("", |l| l.text.as_str())))
        .flatten();
    let mut out = String::from("<table class=\"code blame\">");
    let mut prev = "";
    for l in lines {
        let first = l.commit != prev;
        prev = &l.commit;
        let _ = write!(
            out,
            "<tr id=\"L{}\"{}>",
            l.line_no,
            if first { " class=\"run\"" } else { "" }
        );
        if first {
            let _ = write!(
                out,
                "<td class=\"who\">{} <a class=\"who\" href=\"{}\">{}</a> <span class=\"muted\">{}</span></td>",
                sha_link(&l.commit),
                esc(&author_href(&l.author)),
                esc(&l.author),
                ago(l.time, now)
            );
        } else {
            out.push_str("<td class=\"who\"></td>");
        }
        let _ = write!(
            out,
            "<td class=\"n\"><a href=\"#L{n}\">{n}</a></td><td class=\"c\">{}</td></tr>",
            hl.as_mut()
                .map_or_else(|| esc(&l.text), |h| h.line(&l.text)),
            n = l.line_no
        );
    }
    out.push_str("</table>");
    out
}

/// Link to the history page of one file.
pub(crate) fn file_link(path: &str) -> String {
    crate::fmt::href(&format!("/history/{}", crate::fmt::enc_path(path)))
}

#[cfg(test)]
mod tests {
    use crate::views::{code_html, code_html_first, code_tail_html, crumbs, diff_html};

    const CACHE: usize = 32 << 20;
    use gitterrace_git::{ChangeKind, FileDiff, LineKind};

    #[test]
    fn crumbs_link_prefixes() {
        let c = crumbs("main", "a/b.rs", true);
        assert_eq!(c.len(), 3);
        assert_eq!(c[1].href, "/tree/main/a");
        assert_eq!(c[2].href, "/blob/main/a/b.rs");
    }

    #[test]
    fn code_escapes() {
        let (h, cut) = code_html(CACHE, "x.txt", "<b>\nx");
        assert!(h.contains("&lt;b&gt;"));
        assert!(!cut);
        assert!(h.contains("id=\"L2\""));
        assert!(!h.contains("<table"));
    }

    #[test]
    fn big_files_light_the_first_chunks_and_the_tail_completes_them() {
        let text = "fn f() { let x = 1; }\n".repeat(3_000);
        let (first, _) = code_html_first(CACHE, "a.rs", &text);
        assert!(first.starts_with("<div class=\"code src\" data-hl>"));
        let lit = |h: &str| h.matches("s-keyword").count();
        assert!(lit(&first) > 0 && lit(&first) < 3_000 * 2);
        let (full, _) = code_html(CACHE, "a.rs", &text);
        let tail = code_tail_html(CACHE, "a.rs", &text);
        let head_end = first.match_indices("<div class=\"ch\"").nth(2).unwrap().0;
        let glued = format!(
            "{}{}</div>",
            &full[..full.match_indices("<div class=\"ch\"").nth(2).unwrap().0],
            tail
        );
        assert_eq!(glued.replace(" data-hl", ""), full);
        assert!(first[..head_end].contains("s-keyword"));
        let (small, _) = code_html_first(CACHE, "a.rs", "fn f() {}\n");
        assert!(!small.contains("data-hl"));
    }

    #[tokio::test]
    async fn diff_fragment_out_of_range_is_not_found() {
        use axum::extract::{Path, State};
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path();
        let _fixture = gitterrace_testkit::Fixture::init(p)
            .write(
                "a.txt", "x
",
            )
            .commit("feat: a");
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = std::sync::Arc::new(crate::app::App::new(repo, index));
        let ask = |i: usize| super::diff_fragment(State(app.clone()), Path(("HEAD".into(), i)));
        assert!(ask(0).await.is_ok());
        assert!(ask(99_999).await.is_err());
    }

    #[test]
    fn code_chunks_restart_the_counter() {
        let text = "l\n".repeat(250);
        let (h, _) = code_html(CACHE, "x.txt", &text);
        assert_eq!(h.matches("class=\"ch\"").count(), 3);
        // A reset would open a new scope per chunk and number each one from 1.
        assert!(!h.contains("counter-reset"));
        assert!(h.contains("id=\"L101\" class=\"l\" style=\"counter-set:n 100\""));
        assert!(h.contains("id=\"L250\""));
    }

    #[test]
    fn huge_diffs_fold_behind_a_load_button() {
        use gitterrace_git::{CodeCounts, Hunk, HunkLine};
        let lines = |n: usize| {
            (0..n)
                .map(|i| HunkLine {
                    kind: LineKind::Added,
                    old_no: None,
                    new_no: Some(i as u32 + 1),
                    text: "x".into(),
                })
                .collect::<Vec<_>>()
        };
        let file = |n: usize| FileDiff {
            path: "a.rs".into(),
            old_path: None,
            kind: ChangeKind::Modified,
            binary: false,
            additions: n as u32,
            deletions: 0,
            hunks: vec![Hunk {
                header: "@@".into(),
                lines: lines(n),
            }],
            lines: n as u32,
            rewritten: 0,
            code: CodeCounts::default(),
        };
        let small = diff_html(400, "abc", None, &[file(10)]);
        assert!(!small.contains("data-load-diff"));
        let big = diff_html(400, "abc", None, &[file(401)]);
        assert!(!diff_html(500, "abc", None, &[file(401)]).contains("data-load-diff"));
        assert!(big.contains("data-load-diff=\"/diff/abc/0\""));
        assert!(!big.contains("<div class=\"add\""));
    }
}
