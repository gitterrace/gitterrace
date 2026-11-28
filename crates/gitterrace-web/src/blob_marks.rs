//! Gutter markers on the file, blame and diff views for lines the review basket points at. The
//! lines come from the anchor the API already works out on every read (`FindingAnchor`), so a
//! moved block is marked where it is now, and a block that was edited since it was flagged gets
//! a hollow dot at the span the diff maps it to. A deleted block gets no guessed line.
//!
//! Local servers with the working
//! tree on only: the same gate as the basket, so a public server renders no marker and no
//! note.

use askama::Template;
use gitterrace_types::AnchorStatus;
use gitterrace_types::api::Finding;

use crate::app::App;

/// Longest note shown on hover, in characters. The panel holds the whole note.
const HOVER_CHARS: usize = 200;

/// One finding's lines in the file being shown, 1-based and inclusive.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    id: String,
    path: String,
    start: u32,
    end: u32,
    note: String,
    /// The lines were edited since the finding was made: the dot is hollow.
    edited: bool,
}

/// Which diff a page shows, for [`diff_marks`].
#[derive(Clone, Copy)]
pub(crate) enum DiffAt<'a> {
    /// One commit: its new side is that commit.
    Commit(&'a str),
    /// The whole uncommitted change against HEAD: its new side is the working tree.
    Worktree,
}

/// The link on a mark's first row: to the line, naming the finding, its note on hover.
#[derive(Template)]
#[template(path = "finding_mark.html")]
struct Link<'a> {
    href: String,
    id: &'a str,
    note: String,
    edited: bool,
    label: &'static str,
}

/// The marks for `path` on a page read at `sha`, or from disk when `sha` is `None`. Empty when
/// the basket is off here, unreadable or has nothing on the file.
pub(crate) fn marks(a: &App, sha: Option<&str>, path: &str) -> Vec<Mark> {
    if !crate::gitstate::local_state(a) {
        return Vec::new();
    }
    let Some(api) = a.api() else { return Vec::new() };
    let Ok(all) = api.findings() else { return Vec::new() };
    let head_view = sha.is_none_or(|s| shows_head(a, s, path));
    all.findings.iter().filter_map(|f| mark_of(f, head_view, sha, path)).collect()
}

/// The marks for every file of a diff page. A commit's new side is that commit, so only the
/// findings made at it show, on the lines they were made on. The working tree page compares
/// with the disk, which is what the anchors were checked against, so they show where the lines
/// are now. Empty when the basket is off here.
pub(crate) fn diff_marks(a: &App, at: DiffAt<'_>) -> Vec<Mark> {
    if !crate::gitstate::local_state(a) {
        return Vec::new();
    }
    let Some(api) = a.api() else { return Vec::new() };
    let Ok(all) = api.findings() else { return Vec::new() };
    all.findings
        .iter()
        .filter_map(|f| {
            let path = f.path.as_deref()?;
            match at {
                DiffAt::Commit(id) => mark_of(f, false, Some(id), path),
                DiffAt::Worktree => mark_of(f, true, None, path),
            }
        })
        .collect()
}

/// Whether a page at `sha` shows the lines the anchors speak of: HEAD, with the file the same
/// on disk (an edited file is shown by the working tree view, where its lines are).
fn shows_head(a: &App, sha: &str, path: &str) -> bool {
    if a.repo.resolve("HEAD").ok().as_deref() != Some(sha) {
        return false;
    }
    let limit = a.cfg().limits.raw_mb << 20;
    matches!(gitterrace_git::worktree_blob(&a.repo, path, limit), Ok(None))
}

/// The lines of `f` in `path`: where its anchor says they are now on a page that shows the
/// current text, otherwise the lines it was made on, for a page at that very commit.
fn mark_of(f: &Finding, head_view: bool, sha: Option<&str>, path: &str) -> Option<Mark> {
    let lines = f.lines?;
    let mut edited = false;
    let (start, end) = if head_view {
        if f.anchor.now_path.as_deref() != Some(path) {
            return None;
        }
        match f.anchor.status {
            AnchorStatus::Untouched => {}
            // The place is the diff's mapping: without it there is no line to mark.
            AnchorStatus::LinesEdited if f.anchor.now_lines.is_some() => edited = true,
            _ => return None,
        }
        f.anchor.now_lines.map_or((lines.start, lines.end), |l| (l.start, l.end))
    } else if sha == Some(f.sha.as_str()) && f.path.as_deref() == Some(path) {
        (lines.start, lines.end)
    } else {
        return None;
    };
    Some(Mark { id: f.id.clone(), path: path.to_owned(), start, end, note: f.note.clone(), edited })
}

/// `html` with each marked row classed `fm` and the first row of a finding carrying a link to
/// it. Rows are found by their opening tag, which source text cannot imitate: its `<` is
/// escaped. The link is a plain anchor to the line, so a page without script still lands there.
pub(crate) fn apply(html: String, marks: &[Mark]) -> String {
    const ROW: &str = "<div id=\"L";
    if marks.is_empty() {
        return html;
    }
    let mut out = String::with_capacity(html.len() + marks.len() * 160);
    let mut rest = html.as_str();
    while let Some(at) = rest.find(ROW) {
        let (before, tail) = rest.split_at(at + ROW.len());
        out.push_str(before);
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let n: u32 = tail[..digits].parse().unwrap_or(0);
        let Some(gt) = tail.find('>') else {
            rest = tail;
            continue;
        };
        let (tag, after) = tail.split_at(gt + 1);
        let on_row: Vec<&Mark> = marks.iter().filter(|m| (m.start..=m.end).contains(&n)).collect();
        if on_row.is_empty() {
            out.push_str(tag);
        } else {
            out.push_str(&tag.replacen("class=\"l\"", "class=\"l fm\"", 1));
            for m in on_row.iter().filter(|m| m.start == n) {
                out.push_str(&link(m, format!("#L{n}")));
            }
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn link(m: &Mark, href: String) -> String {
    let note: String = m.note.chars().take(HOVER_CHARS).collect();
    let note = if m.edited { format!("Edited since flagged. {note}") } else { note };
    let label = if m.edited {
        "Open the finding, edited since flagged"
    } else {
        "Open the review finding on this line"
    };
    Link { href, id: &m.id, note, edited: m.edited, label }
        .render()
        .map(|s| s.trim_end().to_owned())
        .unwrap_or_default()
}

/// `html`, the rows of a blame table, with each marked row classed `fm` and the line number
/// cell of a finding's first row carrying the link. A row opens `<tr id="L`, which source text
/// cannot imitate: its `<` is escaped.
pub(crate) fn apply_blame(html: String, marks: &[Mark]) -> String {
    const ROW: &str = "<tr id=\"L";
    const CELL: &str = "<td class=\"n\">";
    if marks.is_empty() {
        return html;
    }
    let mut out = String::with_capacity(html.len() + marks.len() * 160);
    let mut rest = html.as_str();
    while let Some(at) = rest.find(ROW) {
        let (before, tail) = rest.split_at(at + ROW.len());
        out.push_str(before);
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let n: u32 = tail[..digits].parse().unwrap_or(0);
        let Some(gt) = tail.find('>') else {
            rest = tail;
            continue;
        };
        let (tag, after) = tail.split_at(gt + 1);
        let on_row: Vec<&Mark> = marks.iter().filter(|m| (m.start..=m.end).contains(&n)).collect();
        if on_row.is_empty() {
            out.push_str(tag);
            rest = after;
            continue;
        }
        if tag.contains("class=\"run\"") {
            out.push_str(&tag.replacen("class=\"run\"", "class=\"run fm\"", 1));
        } else {
            out.push_str(&tag.replacen('>', " class=\"fm\">", 1));
        }
        let links: String =
            on_row.iter().filter(|m| m.start == n).map(|m| link(m, format!("#L{n}"))).collect();
        match after.find(CELL) {
            Some(c) if !links.is_empty() => {
                let (head, cell_and_after) = after.split_at(c + CELL.len());
                out.push_str(head);
                out.push_str(&links);
                rest = cell_and_after;
            }
            _ => rest = after,
        }
    }
    out.push_str(rest);
    out
}

/// A diff page's `html` with the marked new-side rows of each file classed `fm`. A file is a
/// `<section class="file" id="f{i}" data-path="..."`, which source text cannot imitate.
pub(crate) fn apply_diff(html: String, marks: &[Mark]) -> String {
    // Split so the chrome id scan does not read `f` as a fixed id.
    const SECTION: &str = concat!("<section class=\"file\" id=", "\"f");
    if marks.is_empty() {
        return html;
    }
    let mut out = String::with_capacity(html.len() + marks.len() * 160);
    let mut rest = html.as_str();
    while let Some(at) = rest.find(SECTION) {
        out.push_str(&rest[..at]);
        let from = &rest[at..];
        // The next section, or the end: a section's rows lie between.
        let end = from[SECTION.len()..].find(SECTION).map_or(from.len(), |e| e + SECTION.len());
        let (sec, next) = from.split_at(end);
        let tail = &sec[SECTION.len()..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let path =
            tail.split_once("data-path=\"").and_then(|(_, p)| p.split_once('"')).map(|(p, _)| p);
        let mine: Vec<&Mark> =
            marks.iter().filter(|m| path == Some(crate::fmt::esc(&m.path).as_str())).collect();
        if mine.is_empty() {
            out.push_str(sec);
        } else {
            out.push_str(&diff_rows(sec, &mine, &format!("#f{}", &tail[..digits])));
        }
        rest = next;
    }
    out.push_str(rest);
    out
}

/// The rows of one file's diff `html` with the marks of that file applied. Added and context
/// rows carry a new line number (`data-n`), removed rows have none to mark.
pub(crate) fn diff_rows(html: &str, marks: &[&Mark], href: &str) -> String {
    const ROW: &str = "<div class=\"";
    let mut out = String::with_capacity(html.len() + marks.len() * 160);
    let mut rest = html;
    let mut linked: Vec<&str> = Vec::new();
    while let Some(at) = rest.find(ROW) {
        let (before, tail) = rest.split_at(at + ROW.len());
        out.push_str(before);
        let Some(gt) = tail.find('>') else {
            rest = tail;
            break;
        };
        let (tag, after) = tail.split_at(gt + 1);
        let n = tag
            .split_once("data-n=\"")
            .and_then(|(_, v)| v.split_once('"'))
            .and_then(|(v, _)| v.parse::<u32>().ok());
        let on_row: Vec<&&Mark> = match n {
            Some(n) if tag.starts_with("add\"") || tag.starts_with("ctx\"") => {
                marks.iter().filter(|m| (m.start..=m.end).contains(&n)).collect()
            }
            _ => Vec::new(),
        };
        if on_row.is_empty() {
            out.push_str(tag);
        } else {
            out.push_str(&tag.replacen('"', " fm\"", 1));
            // One link per finding, on its first row that is shown.
            for m in on_row {
                if !linked.contains(&m.id.as_str()) {
                    linked.push(&m.id);
                    out.push_str(&link(m, href.to_owned()));
                }
            }
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use gitterrace_types::AnchorBasis;
    use gitterrace_types::api::{FindingAnchor, FindingLines};

    use super::*;

    fn finding(status: AnchorStatus, now: Option<(u32, u32)>) -> Finding {
        Finding {
            added: 0,
            anchor: FindingAnchor {
                checked_against: AnchorBasis::Head,
                now_lines: now.map(|(start, end)| FindingLines { end, start }),
                now_path: Some("a.txt".to_owned()),
                status,
            },
            id: "f-00000001".to_owned(),
            kind: "line".to_owned(),
            lines: Some(FindingLines { end: 3, start: 2 }),
            note: "n".to_owned(),
            path: Some("a.txt".to_owned()),
            query: None,
            rev: "HEAD".to_owned(),
            sha: "abc".to_owned(),
        }
    }

    fn mark(id: &str, start: u32, end: u32, note: &str, edited: bool) -> Mark {
        Mark {
            id: id.to_owned(),
            path: "a.txt".to_owned(),
            start,
            end,
            note: note.to_owned(),
            edited,
        }
    }

    #[test]
    fn an_edited_block_is_marked_hollow_at_its_mapped_span() {
        let f = finding(AnchorStatus::LinesEdited, Some((5, 7)));
        let m = mark_of(&f, true, None, "a.txt").expect("mark");
        assert_eq!((m.start, m.end, m.edited), (5, 7, true));
        let html = apply("<div id=\"L5\" class=\"l\" data-n=\"5\">x</div>".to_owned(), &[m]);
        assert!(html.contains("class=\"fm-link fe\""), "{html}");
        assert!(html.contains("title=\"Edited since flagged. n\""), "{html}");
    }

    #[test]
    fn an_edited_block_without_a_mapped_place_gets_no_mark() {
        let f = finding(AnchorStatus::LinesEdited, None);
        assert_eq!(mark_of(&f, true, None, "a.txt"), None);
    }

    #[test]
    fn a_blame_row_is_classed_and_the_number_cell_of_the_first_row_links() {
        let html = "<table><tr id=\"L1\" class=\"run\"><td class=\"who\"></td><td class=\"n\">\
                    <a href=\"#L1\">1</a></td></tr><tr id=\"L2\"><td class=\"who\"></td>\
                    <td class=\"n\"><a href=\"#L2\">2</a></td></tr>\
                    <tr id=\"L3\"><td class=\"n\">3</td></tr></table>";
        let out = apply_blame(html.to_owned(), &[mark("f-1", 1, 2, "n", false)]);
        assert!(out.contains("<tr id=\"L1\" class=\"run fm\">"), "{out}");
        assert!(out.contains("<tr id=\"L2\" class=\"fm\">"), "{out}");
        assert!(out.contains("<tr id=\"L3\"><td"), "{out}");
        assert_eq!(out.matches("fm-link").count(), 1, "{out}");
        assert!(out.contains("<td class=\"n\"><a class=\"fm-link\" href=\"#L1\""), "{out}");
    }

    #[test]
    fn a_diff_marks_added_and_context_rows_of_its_file_only() {
        let sec = |i: u32, p: &str| {
            format!(
                "<section class=\"file\" id=\"f{i}\" data-path=\"{p}\"><div class=\"diff\">\
                 <div class=\"del\" data-o=\"2\" data-n=\"\">x</div>\
                 <div class=\"add\" data-o=\"\" data-n=\"2\">y</div>\
                 <div class=\"ctx\" data-o=\"3\" data-n=\"3\">z</div></div></section>"
            )
        };
        let html = format!("{}{}", sec(0, "b.txt"), sec(1, "a.txt"));
        let out = apply_diff(html, &[mark("f-1", 2, 3, "n", false)]);
        assert_eq!(out.matches("class=\"add fm\"").count(), 1, "{out}");
        assert_eq!(out.matches("class=\"ctx fm\"").count(), 1, "{out}");
        assert_eq!(out.matches("class=\"del fm\"").count(), 0, "{out}");
        assert_eq!(out.matches("fm-link").count(), 1, "one link per finding: {out}");
        assert!(out.contains("href=\"#f1\""), "{out}");
        let (first, second) = out.split_once("id=\"f1\"").expect("two sections");
        assert!(!first.contains("fm"), "the other file stays plain: {first}");
        assert!(second.contains("fm"));
    }

    #[test]
    fn a_moved_block_is_marked_where_it_is_now() {
        let f = finding(AnchorStatus::Untouched, Some((5, 6)));
        let m = mark_of(&f, true, None, "a.txt").expect("mark");
        assert_eq!((m.start, m.end), (5, 6));
    }

    #[test]
    fn changed_lines_get_no_guessed_mark() {
        let f = finding(AnchorStatus::LinesChanged, None);
        assert_eq!(mark_of(&f, true, None, "a.txt"), None);
    }

    #[test]
    fn an_old_revision_shows_only_findings_made_at_it() {
        let f = finding(AnchorStatus::Untouched, Some((5, 6)));
        assert_eq!(mark_of(&f, false, Some("def"), "a.txt"), None);
        let m = mark_of(&f, false, Some("abc"), "a.txt").expect("mark");
        assert_eq!((m.start, m.end), (2, 3));
    }

    #[test]
    fn a_row_is_classed_and_the_first_row_links_with_the_note_escaped() {
        let m = mark("f-1", 2, 3, "<b>\"x\"", false);
        let html = "<div id=\"L1\" class=\"l\" data-n=\"1\">a</div>\
                    <div id=\"L2\" class=\"l\" data-n=\"2\">&lt;div id=\"L3\"</div>\
                    <div id=\"L3\" class=\"l\" data-n=\"3\">c</div>";
        let out = apply(html.to_owned(), &[m]);
        assert_eq!(out.matches("class=\"l fm\"").count(), 2, "{out}");
        assert_eq!(out.matches("fm-link").count(), 1, "{out}");
        assert!(out.contains("title=\"&#60;b&#62;&#34;x&#34;\""), "{out}");
        assert!(out.starts_with("<div id=\"L1\" class=\"l\""), "{out}");
    }

    #[test]
    fn no_marks_leave_the_html_as_it_was() {
        let html = "<div id=\"L1\" class=\"l\" data-n=\"1\">a</div>".to_owned();
        assert_eq!(apply(html.clone(), &[]), html);
    }
}
