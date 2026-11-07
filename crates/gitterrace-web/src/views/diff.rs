//! Commit diff HTML: per-file sections, folding budgets and the folded file fragment.

use std::fmt::Write as _;

use gitterrace_git::{ChangeKind, FileDiff, LineKind};

use crate::fmt::{enc_path, enc_seg, esc, esc_text};
use crate::views::{Render, file_label};

fn num(n: Option<u32>) -> String {
    n.map_or_else(String::new, |n| n.to_string())
}

/// A commit's diff without image facts, for tests of the layout.
#[cfg(test)]
pub(crate) fn diff_html(r: &Render, rev: &str, parent: Option<&str>, files: &[FileDiff]) -> String {
    let none = MediaFacts::new();
    diff_html_with(
        r,
        DiffSource::Commit {
            rev,
            parent,
            media: &none,
        },
        files,
    )
}

/// Before and after facts of each changed image, by path: read by the route, which has the
/// repository, so the header can say pixels and bytes instead of `+0 -0`.
pub(crate) type MediaFacts =
    std::collections::HashMap<String, (Option<crate::media::Facts>, Option<crate::media::Facts>)>;

/// Once the files before it changed this many lines in all, later files are folded too.
const TOTAL_DIFF_BUDGET: usize = 2_000;
/// Files with hunks rendered in full before the rest fold. Each file costs a section, a
/// header and at least one chunk whatever its size, so many small files cost as much as one
/// large one. With the line budget this keeps a page near 3,000 line rows and under 30k
/// elements for 2,000 files (scripts/browser-diff-lib.mjs measures it).
const TOTAL_DIFF_FILES: usize = 50;
/// The same budgets in rendered bytes, since a few huge lines cost more than many short ones.
const FILE_DIFF_BYTES: usize = 40_000;
const TOTAL_DIFF_BYTES: usize = 250_000;
/// Lines per `content-visibility` chunk (the stylesheet sizes a chunk as this many rows).
pub(super) const CHUNK_LINES: usize = 100;
/// The words a screen reader says before a changed line, since the line's colour says it to
/// everyone else. Real text, hidden from sight and left out of a copied selection.
const SIGN_ADDED: &str = "<span class=\"vh sign\">Added, </span>";
const SIGN_REMOVED: &str = "<span class=\"vh sign\">Removed, </span>";

/// The hunks of one file as line blocks. Also the body of the fragment a folded file loads.
fn diff_body_html(r: &Render, f: &FileDiff) -> String {
    let mut out = String::from("<div class=\"diffwrap\"><div class=\"diff\">");
    let mut in_chunk = 0usize;
    let total: usize = f
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| l.text.len())
        .sum();
    let plain = total > r.highlight_bytes;
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
            let (cls, sign) = match l.kind {
                LineKind::Added => ("add", SIGN_ADDED),
                LineKind::Removed => ("del", SIGN_REMOVED),
                LineKind::Context => ("ctx", ""),
            };
            let hl = if l.kind == LineKind::Removed {
                old_hl.as_mut()
            } else {
                new_hl.as_mut()
            };
            let body = hl.map_or_else(|| esc_text(&l.text), |h| h.line(&l.text));
            let _ = write!(
                out,
                "<div class=\"{cls}\" data-o=\"{}\" data-n=\"{}\">{sign}{body}</div>",
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

/// Where a page's diff comes from, which decides its links and where a folded file loads from.
#[derive(Clone, Copy)]
pub(crate) enum DiffSource<'a> {
    Commit {
        rev: &'a str,
        parent: Option<&'a str>,
        media: &'a MediaFacts,
    },
    /// The working tree: no blob URLs, folded files load from `/worktree/diff/{path}`.
    Worktree,
}

/// A file folded behind its header: the URL its body loads from and the button's label.
/// `lazy` files (folded only because the page spent its budget) load as they scroll near.
fn fold_button(src: DiffSource<'_>, i: usize, f: &FileDiff, label: &str, lazy: bool) -> String {
    let url = match src {
        DiffSource::Commit { rev, .. } => crate::fmt::href(&format!("/diff/{}/{i}", enc_seg(rev))),
        DiffSource::Worktree => crate::fmt::href(&format!("/worktree/diff/{}", enc_path(&f.path))),
    };
    // A link, so it is reachable by keyboard, works without script and is a page an export
    // crawl finds; nav.js swaps the body in place instead of navigating.
    format!(
        "<p class=\"muted pad fold\"><a class=\"linkbtn\" href=\"{u}\" data-load-diff=\"{u}\"{l} aria-controls=\"f{i}\">{label}</a></p>",
        u = esc(&url),
        l = if lazy { " data-lazy" } else { "" }
    )
}

/// Octal mode as git prints it.
fn mode(m: u16) -> String {
    format!("{m:06o}")
}

/// The one line a file with no hunks shows: a pure rename, a mode change, or neither.
fn no_hunks_line(f: &FileDiff) -> String {
    let mut parts = Vec::new();
    if let (ChangeKind::Renamed, Some(old)) = (f.kind, &f.old_path) {
        parts.push(format!("renamed from <code>{}</code>", esc(old)));
    }
    if let Some((a, b)) = f.modes {
        parts.push(format!("mode changed from {} to {}", mode(a), mode(b)));
    }
    if parts.is_empty() {
        return crate::empty::Empty::new("No textual changes.").to_string();
    }
    let mut line = parts.join(", ");
    if let Some(c) = line.get_mut(..1) {
        c.make_ascii_uppercase();
    }
    format!("<p class=\"muted pad moved\">{line}, no content change.</p>")
}

/// Every file of a page's diff as a section. The first files render in full until the page
/// has spent one of its budgets (files, lines, bytes); every later file, and any file over
/// `r.fold_lines`, is a header whose body loads on demand.
pub(crate) fn diff_html_with(r: &Render, src: DiffSource<'_>, files: &[FileDiff]) -> String {
    let mut out = String::new();
    let mut used = Spent::default();
    for (i, f) in files.iter().enumerate() {
        let _ = write!(
            out,
            "<section class=\"file\" id=\"f{i}\" data-path=\"{p}\" data-h=\"{h}\"><header><span class=\"badge {k}\">{k}</span> ",
            k = f.kind.as_str(),
            p = esc(&f.path),
            h = content_tag(f)
        );
        match src {
            DiffSource::Commit { rev, .. } if f.kind != ChangeKind::Deleted => {
                let _ = write!(
                    out,
                    "<a href=\"{}\">{}</a>",
                    crate::fmt::href(&format!("/blob/{}/{}", enc_seg(rev), enc_path(&f.path))),
                    esc(&file_label(f))
                );
            }
            _ if f.kind == ChangeKind::Deleted => out.push_str(&esc(&f.path)),
            _ => out.push_str(&esc(&file_label(f))),
        }
        let facts = match src {
            DiffSource::Commit { media, .. } if f.binary => media.get(&f.path),
            _ => None,
        };
        if let Some((old, new)) = facts {
            out.push_str(&crate::media::diff_counts(*old, *new));
            out.push_str("</header>");
        } else {
            let _ = write!(
                out,
                "<span class=\"counts\">{}</span></header>",
                crate::delta::line_delta(f.additions, f.deletions)
            );
        }
        out.push_str(&file_body(r, src, i, f, &mut used));
        out.push_str("</section>");
    }
    out
}

/// A short tag of a file's diff content. `live.js` carries a loaded body over a live refresh
/// only while the tag is unchanged, and loads it again when it differs.
fn content_tag(f: &FileDiff) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    f.path.hash(&mut h);
    f.additions.hash(&mut h);
    f.deletions.hash(&mut h);
    for hunk in &f.hunks {
        hunk.header.hash(&mut h);
        for l in &hunk.lines {
            (l.kind as u8, l.old_no, l.new_no, &l.text).hash(&mut h);
        }
    }
    format!("{:x}", h.finish())
}

/// What the files rendered in full so far cost.
#[derive(Default)]
struct Spent {
    files: usize,
    lines: usize,
    bytes: usize,
}

fn file_body(r: &Render, src: DiffSource<'_>, i: usize, f: &FileDiff, used: &mut Spent) -> String {
    use crate::media::Side;
    if f.binary {
        let shown =
            crate::media::is_image(&f.path) && crate::features::on(gitterrace_config::Feature::Raw);
        let old_path = f.old_path.as_deref().unwrap_or(&f.path);
        let sides = match src {
            DiffSource::Commit { rev, parent, .. } => Some((
                parent.map(|p| Side::At(p, old_path)),
                Side::At(rev, f.path.as_str()),
            )),
            // HEAD through `/raw`, the copy on disk through `/worktree/raw`.
            DiffSource::Worktree => {
                Some((Some(Side::At("HEAD", old_path)), Side::Worktree(&f.path)))
            }
        };
        if let (true, Some((old, new))) = (shown, sides) {
            let old = old.filter(|_| f.kind != ChangeKind::Added);
            let new = (f.kind != ChangeKind::Deleted).then_some(new);
            return crate::media::diff_html(old, new);
        }
        return "<p class=\"muted pad\">Binary file not shown.</p>".to_owned();
    }
    if f.hunks.is_empty() {
        return no_hunks_line(f);
    }
    let changed = (f.additions + f.deletions) as usize;
    let bytes: usize = f
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| l.text.len() + 48)
        .sum();
    let alone = changed > r.fold_lines || bytes > FILE_DIFF_BYTES;
    let page_full = used.files >= TOTAL_DIFF_FILES
        || used.lines + changed > TOTAL_DIFF_BUDGET
        || used.bytes + bytes > TOTAL_DIFF_BYTES;
    // Counted even when folded: once the page is past a budget, every later file folds, so
    // the first screen is the same files whatever lies further down.
    used.files += 1;
    used.lines += changed;
    used.bytes += bytes;
    if alone || page_full {
        fold_button(
            src,
            i,
            f,
            &format!(
                "Load diff ({} changed)",
                crate::fmt::count(changed, "line", "lines")
            ),
            !alone,
        )
    } else {
        diff_body_html(r, f)
    }
}

/// `GET /diff/{rev}/{index}`: the body of one folded file of a commit, as a fragment.
pub(crate) async fn diff_fragment(
    axum::extract::State(app): axum::extract::State<std::sync::Arc<crate::app::App>>,
    axum::extract::Path((rev, index)): axum::extract::Path<(String, usize)>,
) -> crate::error::Result<axum::response::Html<String>> {
    let html = crate::app::blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let r = Render::of(a);
        Ok(a.repo
            .commit_file(&id, index)?
            .as_ref()
            .map(|f| diff_body_html(&r, f)))
    })
    .await?;
    html.map(axum::response::Html)
        .ok_or_else(|| gitterrace_git::Error::PathNotFound(format!("file {index}")).into())
}

/// `GET /worktree/diff/{path}`: the body of one folded file of the working tree page.
pub(crate) async fn worktree_fragment(
    axum::extract::State(app): axum::extract::State<std::sync::Arc<crate::app::App>>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> crate::error::Result<axum::response::Html<String>> {
    let html = crate::app::blocking(&app, move |a| {
        let r = Render::of(a);
        Ok(
            gitterrace_git::worktree_file_diff(&a.repo, &path)?.map(|f| {
                if f.hunks.is_empty() {
                    no_hunks_line(&f)
                } else {
                    diff_body_html(&r, &f)
                }
            }),
        )
    })
    .await?;
    // A file committed or reverted since the page loaded has nothing left to show.
    Ok(axum::response::Html(html.unwrap_or_else(|| {
        crate::empty::Empty::new("No longer changed.").to_string()
    })))
}

#[cfg(test)]
mod tests;
