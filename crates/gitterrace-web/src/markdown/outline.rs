//! The "On this page" outline: the headings a document lists, as a rail and as a menu.

use askama::Template;

use crate::symbols::SymbolKind;

/// One heading of a document: level 1 to 6, its slug (before `crate::markdown::ids::doc_id`) and
/// text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutlineItem {
    pub(crate) level: u8,
    pub(crate) id: String,
    pub(crate) text: String,
    /// A section of the page itself (an insights section), whose id is the page's own and is
    /// linked as is; a document heading's id goes through `doc_id`.
    pub(crate) chrome: bool,
    /// What a source file's symbol declares; `None` for a heading or a page section.
    pub(crate) kind: Option<SymbolKind>,
}

impl OutlineItem {
    /// A document heading, whose id goes through `doc_id`.
    pub(crate) fn heading(level: u8, id: String, text: &str) -> Self {
        Self {
            level,
            id,
            text: text.to_owned(),
            chrome: false,
            kind: None,
        }
    }
}

/// Fewest listed headings that make an outline worth showing.
const OUTLINE_MIN: usize = 3;

/// The headings an outline lists: h2 and h3, plus h1 only when the document has several.
/// Empty when fewer than three remain.
pub(crate) fn outline_items(all: &[OutlineItem]) -> Vec<&OutlineItem> {
    let h1s = all.iter().filter(|h| h.level == 1).count();
    let kept: Vec<&OutlineItem> = all
        .iter()
        .filter(|h| (2..=3).contains(&h.level) || (h.level == 1 && h1s > 1))
        .collect();
    if kept.len() < OUTLINE_MIN {
        Vec::new()
    } else {
        kept
    }
}

/// One listed heading, as the template shows it: indent 0 or 1, the target id and the text.
struct OutlineLink<'a> {
    depth: u8,
    id: String,
    text: &'a str,
    /// The short kind label and its spoken word, for a source file's symbol.
    kind: Option<(&'static str, &'static str)>,
}

/// The markup, from `outline.html`: the only place that writes an outline.
/// `escape = "svg"` picks the crate's `& < > "` escaper, the bytes `fmt::esc` wrote before.
#[derive(Template)]
#[template(path = "outline.html", escape = "svg")]
struct OutlineMarkup<'a> {
    rail: bool,
    icon_only: bool,
    links: Vec<OutlineLink<'a>>,
    /// The widest kind label, in characters: the label column's width. 0 without kinds.
    kind_width: usize,
}

/// The outline as the rail or the menu, empty below three headings.
fn outline_html(all: &[OutlineItem], rail: bool, icon_only: bool) -> String {
    let items = outline_items(all);
    if items.is_empty() {
        return String::new();
    }
    let top = items.iter().map(|h| h.level).min().unwrap_or(2);
    let links = items
        .iter()
        .map(|h| OutlineLink {
            depth: u8::from(h.level > top),
            id: if h.chrome {
                h.id.clone()
            } else {
                crate::markdown::ids::doc_id(&h.id)
            },
            text: &h.text,
            kind: h.kind.map(|k| (k.label(), k.spoken())),
        })
        .collect::<Vec<_>>();
    let kind_width = links
        .iter()
        .filter_map(|l| l.kind.map(|k| k.0.len()))
        .max()
        .unwrap_or(0);
    OutlineMarkup {
        rail,
        icon_only,
        links,
        kind_width,
    }
    .render()
    .unwrap_or_default()
}

/// The wide-screen rail, "On this page". Empty below three headings.
pub(crate) fn outline_rail(all: &[OutlineItem]) -> String {
    outline_html(all, true, false)
}

/// The narrow-screen menu: a native `<details>`, so it opens without script. Empty below three
/// headings. `icon_only` drops the visible text for a button in a header bar: the label moves to
/// `aria-label` and `data-tip`.
pub(crate) fn outline_menu(all: &[OutlineItem], icon_only: bool) -> String {
    outline_html(all, false, icon_only)
}

/// The outline items of the sections a page shows: `(id, label, shown)`, in page order. A
/// section the page leaves out (a feature switched off, nothing to show) is not listed.
pub(crate) fn section_jumps(sections: &[(&str, &str, bool)]) -> Vec<OutlineItem> {
    sections
        .iter()
        .filter(|(_, _, shown)| *shown)
        .map(|(id, text, _)| OutlineItem {
            level: 2,
            id: (*id).to_owned(),
            text: (*text).to_owned(),
            chrome: true,
            kind: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No outline renders a search field, however long: headings or a source file's symbols.
    #[test]
    fn outline_has_no_input() {
        let heads: Vec<OutlineItem> = (0..30)
            .map(|i| OutlineItem::heading(2, format!("h{i}"), &format!("Head {i}")))
            .collect();
        let syms: Vec<OutlineItem> = (0..30)
            .map(|i| OutlineItem {
                kind: Some(SymbolKind::Function),
                ..OutlineItem::heading(2, format!("f{i}"), &format!("f{i}"))
            })
            .collect();
        for items in [&heads, &syms] {
            for rail in [true, false] {
                let html = outline_html(items, rail, false);
                assert!(html.contains("<ol"), "the list renders");
                assert!(!html.contains("<input"), "no search field: {html}");
            }
        }
    }

    #[test]
    fn only_shown_sections_are_listed_in_order() {
        let j = section_jumps(&[("a", "A", true), ("b", "B", false), ("c", "C", true)]);
        let ids: Vec<_> = j.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"]);
    }

    #[test]
    fn the_menu_links_each_section_and_needs_three() {
        let three = section_jumps(&[("a", "A", true), ("types", "T", true), ("c", "C", true)]);
        let menu = outline_menu(&three, false);
        assert!(menu.contains("On this page"));
        // A page's own section ids are linked as they are, even where a document's would be
        // prefixed.
        for id in ["a", "types", "c"] {
            assert!(menu.contains(&format!("href=\"#{id}\"")), "{id}");
        }
        let two = section_jumps(&[("a", "A", true), ("c", "C", true)]);
        assert_eq!(outline_menu(&two, false).len(), 0);
    }
}
