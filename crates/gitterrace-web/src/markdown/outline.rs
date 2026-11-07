//! The "On this page" outline: the headings a document lists, as a rail and as a menu.

use askama::Template;

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
}

impl OutlineItem {
    /// A document heading, whose id goes through `doc_id`.
    pub(crate) fn heading(level: u8, id: String, text: &str) -> Self {
        Self {
            level,
            id,
            text: text.to_owned(),
            chrome: false,
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
}

/// The markup, from `outline.html`: the only place that writes an outline.
/// `escape = "svg"` picks the crate's `& < > "` escaper, the bytes `fmt::esc` wrote before.
#[derive(Template)]
#[template(path = "outline.html", escape = "svg")]
struct OutlineMarkup<'a> {
    rail: bool,
    icon_only: bool,
    links: Vec<OutlineLink<'a>>,
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
        })
        .collect();
    OutlineMarkup {
        rail,
        icon_only,
        links,
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
