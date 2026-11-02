//! Commit diff HTML: per-file sections, folding budgets and the folded file fragment.

use std::fmt::Write as _;

use gitterrace_git::{ChangeKind, FileDiff, LineKind};

use crate::fmt::{enc_path, enc_seg, esc, esc_text};
use crate::views::{file_label, kind_name};

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
pub(super) const CHUNK_LINES: usize = 100;

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
