//! Gutter markers on the file view for lines the review basket points at. The lines come from
//! the anchor the API already works out on every read (`FindingAnchor`), so a moved block is
//! marked where it is now. Local servers with the working tree on only: the same gate as the
//! basket, so a public server renders no marker and no note.

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
    start: u32,
    end: u32,
    note: String,
}

/// The link on a mark's first row: to the line, naming the finding, its note on hover.
#[derive(Template)]
#[template(path = "finding_mark.html")]
struct Link<'a> {
    line: u32,
    id: &'a str,
    note: String,
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
    let (start, end) = if head_view {
        if f.anchor.status != AnchorStatus::Untouched || f.anchor.now_path.as_deref() != Some(path)
        {
            return None;
        }
        f.anchor.now_lines.map_or((lines.start, lines.end), |l| (l.start, l.end))
    } else if sha == Some(f.sha.as_str()) && f.path.as_deref() == Some(path) {
        (lines.start, lines.end)
    } else {
        return None;
    };
    Some(Mark { id: f.id.clone(), start, end, note: f.note.clone() })
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
                out.push_str(&link(m, n));
            }
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn link(m: &Mark, line: u32) -> String {
    let note: String = m.note.chars().take(HOVER_CHARS).collect();
    Link { line, id: &m.id, note }.render().map(|s| s.trim_end().to_owned()).unwrap_or_default()
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
        let m = Mark { id: "f-1".to_owned(), start: 2, end: 3, note: "<b>\"x\"".to_owned() };
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
