//! Source line spans on rendered blocks: `data-line="a-b"` on paragraphs, headings, list items,
//! quotes, tables and code, so text marked in the preview maps back to the raw lines.
//!
//! Tags the HTML writer emits itself get a sentinel event right after their start tag, which
//! [`place`] folds into the tag. The sentinel keeps the writer's newline state as the tag left
//! it, so stripping the attribute gives byte for byte the HTML rendered without spans.

use std::ops::Range;

use pulldown_cmark::Event;

/// Brackets a span in the rendered output, up to the sanitizer and past it. Repository text
/// never holds these: [`neutralize`] replaces them before parsing.
const OPEN: char = '\u{E000}';
const CLOSE: char = '\u{E001}';

/// Byte offsets of the parsed text to 1-based lines of the whole file.
pub(super) struct LineMap {
    starts: Vec<usize>,
    /// Lines before the parsed text, the front matter.
    skip: usize,
}

impl LineMap {
    pub(super) fn new(body: &str, skip: usize) -> Self {
        let starts = std::iter::once(0)
            .chain(body.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self { starts, skip }
    }

    fn line(&self, at: usize) -> usize {
        self.skip + self.starts.partition_point(|&s| s <= at)
    }

    /// `a-b` for a block's byte range, its trailing line ends left out.
    pub(super) fn span(&self, body: &str, r: &Range<usize>) -> String {
        let bytes = body.as_bytes();
        let mut end = r.end.min(bytes.len());
        while end > r.start + 1 && matches!(bytes[end - 1], b'\n' | b'\r') {
            end -= 1;
        }
        let last = end.saturating_sub(1).max(r.start);
        format!("{}-{}", self.line(r.start), self.line(last))
    }
}

/// The source with the sentinel characters replaced by U+FFFD. All three are three bytes in
/// UTF-8, so every offset the parser reports still points into the original file.
pub(super) fn neutralize(src: &str) -> std::borrow::Cow<'_, str> {
    if src.contains([OPEN, CLOSE]) {
        std::borrow::Cow::Owned(src.replace([OPEN, CLOSE], "\u{FFFD}"))
    } else {
        std::borrow::Cow::Borrowed(src)
    }
}

/// The sentinel for a tag the renderer builds itself, written right before that tag.
pub(super) fn mark(span: &str) -> String {
    format!("{OPEN}{span}{CLOSE}")
}

/// The sentinel for a tag the HTML writer emits, as an event right before its start event.
/// It ends in a line break, so the writer acts as if a line had just ended; [`place`] keeps
/// that break only where the writer would have written one itself.
pub(super) fn sentinel<'a>(span: &str) -> Event<'a> {
    Event::InlineHtml(format!("{OPEN}{span}{CLOSE}\n").into())
}

/// Folds every sentinel into the start tag that follows it, as `data-line="a-b"`. Runs on the
/// sanitized HTML, which holds no `data-line` of the repository's and no sentinel but ours.
pub(super) fn place(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + html.len() / 8);
    let mut rest = html;
    while let Some(i) = rest.find(OPEN) {
        out.push_str(&rest[..i]);
        rest = &rest[i + OPEN.len_utf8()..];
        let Some(span) = rest.find(CLOSE).map(|j| &rest[..j]).filter(|s| valid(s)) else {
            continue;
        };
        rest = &rest[span.len() + CLOSE.len_utf8()..];
        if let Some(after) = rest.strip_prefix('\n') {
            // The writer opens a block with a line break unless one was just written.
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            rest = after;
        }
        let Some(end) = tag_end(rest) else { continue };
        out.push_str(&rest[..end]);
        out.push_str(" data-line=\"");
        out.push_str(span);
        out.push('"');
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Where the start tag at the front of `s` closes, at its `>` (or `/>`), quotes respected.
fn tag_end(s: &str) -> Option<usize> {
    if !s.starts_with('<') || s.starts_with("</") {
        return None;
    }
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '>' if !quoted => return Some(if s[..i].ends_with('/') { i - 1 } else { i }),
            _ => {}
        }
    }
    None
}

/// `a-b`, two line numbers, the only form the sanitizer lets through.
pub(super) fn valid(span: &str) -> bool {
    let num = |n: &str| !n.is_empty() && n.len() <= 9 && n.bytes().all(|c| c.is_ascii_digit());
    span.split_once('-').is_some_and(|(a, b)| num(a) && num(b))
}

#[cfg(test)]
mod tests {
    use super::{LineMap, place, valid};
    use crate::markdown::{Base, render_with};

    fn render(src: &str) -> String {
        render_with(src, Base::new("HEAD", ""))
    }

    /// Every block names its own lines of the file, front matter counted.
    #[test]
    fn blocks_carry_their_source_lines() {
        let src = "---\ntitle: x\n---\n# Head `code`\n\nA wrapped [link](a.md)\nparagraph with ``a \
            ` b``.\n\n- one\n- two\n\n> quoted\n> twice\n\n```rs\nfn f() {}\n```\n\n| a \
            |\n|---|\n| 1 |\n";
        let out = render(src);
        for want in [
            "<h1 id=\"head-code\" data-line=\"4-4\">",
            "<p data-line=\"6-7\">A wrapped",
            "<li data-line=\"9-9\">one",
            "<li data-line=\"10-10\">two",
            "<blockquote data-line=\"12-13\">",
            "<pre data-line=\"15-17\">",
            "<table data-line=\"19-21\">",
        ] {
            assert!(out.contains(want), "{want} in {out}");
        }
    }

    /// A span sits inside its tag and leaves the writer's line breaks where they were.
    #[test]
    fn spans_keep_the_writer_layout() {
        let out = render("> a\n\n> [!NOTE]\n> b\n\n1. x\n\n   y\n\npara\n");
        assert!(
            out.contains(
                "<blockquote data-line=\"1-1\">\n<p data-line=\"1-1\">a</p>\n</blockquote>"
            ),
            "{out}"
        );
        assert!(
            out.contains("markdown-alert-note\" data-line=\"3-4\">"),
            "{out}"
        );
        assert!(out.contains("<p data-line=\"10-10\">para</p>"), "{out}");
    }

    /// Neither repository HTML nor the sentinel characters can put a span on the page.
    #[test]
    fn forged_spans_are_dropped() {
        let out = render("<p data-line=\"5-6\">a</p>\n\n<span data-line=\"7-8\">b</span>\n");
        assert!(!out.contains("5-6") && !out.contains("7-8"), "{out}");
        let out = render("x \u{E000}9-9\u{E001}<b>y</b> \u{E000}\n\nz\n");
        assert!(!out.contains("data-line=\"9-9\""), "{out}");
        assert!(!out.contains(['\u{E000}', '\u{E001}']), "{out}");
        assert!(
            out.contains("<p data-line=\"1-1\">x \u{FFFD}9-9\u{FFFD}<b>"),
            "{out}"
        );
    }

    #[test]
    fn spans_skip_front_matter_and_trailing_newlines() {
        let body = "one\ntwo\n\nthree\n";
        let m = LineMap::new(body, 3);
        assert_eq!(m.span(body, &(0..8)), "4-5");
        assert_eq!(m.span(body, &(9..15)), "7-7");
    }

    #[test]
    fn sentinels_fold_into_the_next_tag() {
        let raw = "\u{E000}1-2\u{E001}\n<p>a</p>\nx\u{E000}3-4\u{E001}\n<h1 id=\"a>b\">t</h1>";
        assert_eq!(
            place(raw),
            "<p data-line=\"1-2\">a</p>\nx\n<h1 id=\"a>b\" data-line=\"3-4\">t</h1>"
        );
        assert!(valid("12-30") && !valid("1-") && !valid("a-2") && !valid("1"));
    }
}
