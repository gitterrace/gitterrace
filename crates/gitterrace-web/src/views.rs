//! View models and the HTML fragments too hot to loop over in templates.

use gitterrace_git::{
    BlameLine, ChangeKind, CommitDetail, CommitSummary, FileDiff, LineKind, RefInfo, TreeEntry,
    TreeEntryKind,
};

use crate::fmt::{ago, date, enc, esc, esc_text, href, short};

const MAX_BLOB_LINES: usize = 20_000;
/// Largest text blob the browser builds a page for; bigger ones link to the raw bytes.
pub(crate) const MAX_BLOB_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) struct Nav {
    pub(crate) href: String,
    pub(crate) label: &'static str,
    pub(crate) on: bool,
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
}

pub(crate) struct Crumb {
    pub(crate) name: String,
    pub(crate) href: String,
}

pub(crate) struct CommitRow {
    pub(crate) short: String,
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
    chrome_at(app, active, "HEAD")
}

pub(crate) fn chrome_at(app: &crate::app::App, active: &str, rev: &str) -> Chrome {
    let items = [
        ("/", "Code", "files"),
        ("/log/HEAD", "Commits", "log"),
        ("/refs", "Branches & Tags", "refs"),
        ("/stats", "Insights", "stats"),
    ];
    Chrome {
        base: crate::fmt::base(),
        name: app.name.clone(),
        branch: app.repo.head_branch().unwrap_or_else(|| "HEAD".to_owned()),
        md: false,
        rev: rev.to_owned(),
        rev_enc: enc(rev, false),
        commit: app.repo.resolve(rev).unwrap_or_default(),
        dirty: app.dirty(),
        nav: items
            .into_iter()
            .map(|(path, label, key)| Nav {
                href: href(path),
                label,
                on: key == active,
            })
            .collect(),
    }
}

pub(crate) fn stats_tabs(active: &str) -> Vec<Nav> {
    [
        ("/stats", "Activity", "a"),
        ("/stats/files", "Hot files", "f"),
        ("/stats/types", "Commit types", "t"),
        ("/stats/languages", "Languages", "l"),
        ("/stats/frequency", "Code frequency", "cf"),
        ("/stats/contributors", "Contributors", "co"),
    ]
    .into_iter()
    .map(|(path, label, key)| Nav {
        href: href(path),
        label,
        on: key == active,
    })
    .collect()
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
    href(&format!("/log/HEAD?author={}", enc(email, false)))
}

pub(crate) fn commit_row(c: &CommitSummary, now: i64) -> CommitRow {
    CommitRow {
        short: short(&c.id),
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
    let r = enc(rev, false);
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
            href: href(&format!("/{kind}/{r}/{}", enc(&acc, true))),
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
            let full = enc(&format!("{base}{}", e.name), true);
            let r = enc(rev, false);
            let (href, kind) = match e.kind {
                TreeEntryKind::Dir => (href(&format!("/tree/{r}/{full}")), "dir"),
                TreeEntryKind::File => (href(&format!("/blob/{r}/{full}")), "file"),
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
            log: href(&format!("/log/{}", enc(&r.name, false))),
            commit: short(&r.target),
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

fn kind_name(k: ChangeKind) -> &'static str {
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

pub(crate) fn diff_html(rev: &str, files: &[FileDiff]) -> String {
    diff_html_with(Some(rev), files)
}

/// A file whose changed lines pass this is folded behind a "Load diff" button.
const FILE_DIFF_BUDGET: usize = 400;
/// Once the files before it changed this many lines in all, later files are folded too.
const TOTAL_DIFF_BUDGET: usize = 2_000;
/// The same budgets in rendered bytes, since a few huge lines cost more than many short ones.
const FILE_DIFF_BYTES: usize = 40_000;
const TOTAL_DIFF_BYTES: usize = 250_000;
/// Lines per `content-visibility` chunk (the stylesheet sizes a chunk as this many rows).
const CHUNK_LINES: usize = 100;

/// The hunks of one file as line blocks. Also the body of the fragment a folded file loads.
pub(crate) fn diff_body_html(f: &FileDiff) -> String {
    let mut out = String::from("<div class=\"diffwrap\"><div class=\"diff\">");
    let mut in_chunk = 0usize;
    for h in &f.hunks {
        if in_chunk == 0 {
            out.push_str("<div class=\"ch\">");
        }
        crate::fmt::push(
            &mut out,
            &format!("<div class=\"hh\">{}</div>", esc(&h.header)),
        );
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
            crate::fmt::push(
                &mut out,
                &format!(
                    "<div class=\"{cls}\" data-o=\"{}\" data-n=\"{}\">{}</div>",
                    num(l.old_no),
                    num(l.new_no),
                    esc_text(&l.text)
                ),
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

/// `rev` of `None` renders file names without links (the worktree has no blob URLs).
pub(crate) fn diff_html_with(rev: Option<&str>, files: &[FileDiff]) -> String {
    let mut out = String::new();
    let mut budget_used = 0usize;
    let mut bytes_used = 0usize;
    for (i, f) in files.iter().enumerate() {
        crate::fmt::push(
            &mut out,
            &format!(
                "<section class=\"file\" id=\"f{i}\"><header><span class=\"badge {k}\">{k}</span> ",
                k = kind_name(f.kind)
            ),
        );
        if f.kind == ChangeKind::Deleted {
            out.push_str(&esc(&f.path));
        } else if let Some(rev) = rev {
            crate::fmt::push(
                &mut out,
                &format!(
                    "<a href=\"{}/blob/{}/{}\">{}</a>",
                    crate::fmt::base(),
                    enc(rev, false),
                    enc(&f.path, true),
                    esc(&file_label(f))
                ),
            );
        } else {
            out.push_str(&esc(&file_label(f)));
        }
        crate::fmt::push(
            &mut out,
            &format!(
                "<span class=\"counts\"><b class=\"add\">+{}</b> <b class=\"del\">-{}</b></span></header>",
                crate::fmt::thousands(f.additions),
                crate::fmt::thousands(f.deletions)
            ),
        );
        if f.binary {
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
                && (changed > FILE_DIFF_BUDGET
                    || bytes > FILE_DIFF_BYTES
                    || budget_used + changed > TOTAL_DIFF_BUDGET
                    || bytes_used + bytes > TOTAL_DIFF_BYTES);
            budget_used += changed;
            bytes_used += bytes;
            if let (true, Some(rev)) = (over, rev) {
                crate::fmt::push(
                    &mut out,
                    &format!(
                        "<p class=\"muted pad\"><button type=\"button\" class=\"linkbtn\" data-load-diff=\"{}/diff/{}/{i}\">Load diff ({} changed lines)</button></p>",
                        crate::fmt::base(),
                        enc(rev, false),
                        crate::fmt::thousands(changed)
                    ),
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
        let d = a.repo.commit(&id)?;
        Ok(d.files.get(index).map(diff_body_html).unwrap_or_default())
    })
    .await?;
    Ok(axum::response::Html(html))
}

/// Returns the line blocks and whether it was cut at the line cap.
/// One div per line: the number is a CSS counter, so it neither selects nor copies.
pub(crate) fn code_html(text: &str) -> (String, bool) {
    let mut out = String::with_capacity(text.len() + text.len() / 8 + 64);
    out.push_str("<div class=\"code src\">");
    let mut cut = false;
    let mut open = false;
    for (i, line) in text.lines().enumerate() {
        if i >= MAX_BLOB_LINES {
            cut = true;
            break;
        }
        if i % CHUNK_LINES == 0 {
            if open {
                out.push_str("</div>");
            }
            crate::fmt::push(
                &mut out,
                &format!("<div class=\"ch\" style=\"counter-reset:n {i}\">"),
            );
            open = true;
        }
        crate::fmt::push(
            &mut out,
            &format!(
                "<div id=\"L{}\" class=\"l\">{}</div>",
                i + 1,
                esc_text(line)
            ),
        );
    }
    if open {
        out.push_str("</div>");
    }
    out.push_str("</div>");
    (out, cut)
}

pub(crate) fn blame_html(lines: &[BlameLine], now: i64) -> String {
    let mut out = String::from("<table class=\"code blame\">");
    let mut prev = "";
    for l in lines {
        let first = l.commit != prev;
        prev = &l.commit;
        crate::fmt::push(
            &mut out,
            &format!(
                "<tr id=\"L{}\"{}>",
                l.line_no,
                if first { " class=\"run\"" } else { "" }
            ),
        );
        if first {
            crate::fmt::push(
                &mut out,
                &format!(
                    "<td class=\"who\"><a href=\"{}/commit/{}\">{}</a> <a class=\"who\" href=\"{}\">{}</a> <span class=\"muted\">{}</span></td>",
                    crate::fmt::base(),
                    esc(&l.commit),
                    esc(&short(&l.commit)),
                    esc(&author_href(&l.author)),
                    esc(&l.author),
                    ago(l.time, now)
                ),
            );
        } else {
            out.push_str("<td class=\"who\"></td>");
        }
        crate::fmt::push(
            &mut out,
            &format!(
                "<td class=\"n\"><a href=\"#L{n}\">{n}</a></td><td class=\"c\">{}</td></tr>",
                esc(&l.text),
                n = l.line_no
            ),
        );
    }
    out.push_str("</table>");
    out
}

#[cfg(test)]
mod tests {
    use crate::views::{code_html, crumbs, diff_html};
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
        let (h, cut) = code_html("<b>\nx");
        assert!(h.contains("&lt;b&gt;"));
        assert!(!cut);
        assert!(h.contains("id=\"L2\""));
        assert!(!h.contains("<table"));
    }

    #[test]
    fn code_chunks_restart_the_counter() {
        let text = "l\n".repeat(250);
        let (h, _) = code_html(&text);
        assert_eq!(h.matches("class=\"ch\"").count(), 3);
        assert!(h.contains("counter-reset:n 100"));
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
        let small = diff_html("abc", &[file(10)]);
        assert!(!small.contains("data-load-diff"));
        let big = diff_html("abc", &[file(401)]);
        assert!(big.contains("data-load-diff=\"/diff/abc/0\""));
        assert!(!big.contains("<div class=\"add\""));
    }
}
