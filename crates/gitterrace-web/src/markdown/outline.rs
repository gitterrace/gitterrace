//! The "On this page" outline: the headings a document lists, as a rail and as a menu.

use crate::fmt::esc;

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

fn outline_list(items: &[&OutlineItem]) -> String {
    let top = items.iter().map(|h| h.level).min().unwrap_or(2);
    let mut out = String::from("<ol>");
    for h in items {
        out.push_str("<li class=\"d");
        out.push(if h.level > top { '1' } else { '0' });
        out.push_str("\"><a href=\"#");
        if h.chrome {
            out.push_str(&esc(&h.id));
        } else {
            out.push_str(&esc(&crate::markdown::ids::doc_id(&h.id)));
        }
        out.push_str("\" title=\"");
        out.push_str(&esc(&h.text));
        out.push_str("\">");
        out.push_str(&esc(&h.text));
        out.push_str("</a></li>");
    }
    out.push_str("</ol>");
    out
}

/// The wide-screen rail, "On this page". Empty below three headings.
pub(crate) fn outline_rail(all: &[OutlineItem]) -> String {
    let items = outline_items(all);
    if items.is_empty() {
        return String::new();
    }
    format!(
        "<nav class=\"outline rail\" aria-label=\"On this page\" data-outline>\
         <h2>On this page</h2>{}</nav>",
        outline_list(&items)
    )
}

/// The narrow-screen menu: a native `<details>`, so it opens without script. Empty below three
/// headings. `icon_only` drops the visible text for a button in a header bar: the label moves to
/// `aria-label` and `data-tip`.
pub(crate) fn outline_menu(all: &[OutlineItem], icon_only: bool) -> String {
    let items = outline_items(all);
    if items.is_empty() {
        return String::new();
    }
    let button = if icon_only {
        "<summary class=\"outlinebtn icon\" aria-label=\"On this page\" data-tip=\"On this page\"></summary>"
    } else {
        "<summary class=\"outlinebtn\">On this page</summary>"
    };
    format!(
        "<details class=\"outline menu\" data-outline>{button}\
         <div class=\"outline-pop\"><input type=\"search\" class=\"outline-q\" \
         placeholder=\"Filter headings\" aria-label=\"Filter headings\" hidden>{}\
         <p class=\"muted outline-none\" hidden>No matching heading</p></div></details>",
        outline_list(&items)
    )
}
