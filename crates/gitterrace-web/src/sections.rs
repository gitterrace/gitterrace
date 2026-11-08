//! "On this page" for insights pages that stack several sections: the same outline menu a
//! document gets, fed with the page's own section ids instead of Markdown headings.

use crate::markdown::OutlineItem;

/// The outline items of the sections a page shows: `(id, label, shown)`, in page order. A
/// section the page leaves out (a feature switched off, nothing to show) is not listed.
pub(crate) fn jumps(sections: &[(&str, &str, bool)]) -> Vec<OutlineItem> {
    sections
        .iter()
        .filter(|(_, _, shown)| *shown)
        .map(|(id, text, _)| OutlineItem {
            level: 2,
            id: (*id).to_owned(),
            text: (*text).to_owned(),
            chrome: true,
            symbol: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_shown_sections_are_listed_in_order() {
        let j = jumps(&[("a", "A", true), ("b", "B", false), ("c", "C", true)]);
        let ids: Vec<_> = j.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"]);
    }

    #[test]
    fn the_menu_links_each_section_and_needs_three() {
        let three = jumps(&[("a", "A", true), ("types", "T", true), ("c", "C", true)]);
        let menu = crate::markdown::outline_menu(&three, false);
        assert!(menu.contains("On this page"));
        // A page's own section ids are linked as they are, even where a document's would be
        // prefixed.
        for id in ["a", "types", "c"] {
            assert!(menu.contains(&format!("href=\"#{id}\"")), "{id}");
        }
        let two = jumps(&[("a", "A", true), ("c", "C", true)]);
        assert!(crate::markdown::outline_menu(&two, false).is_empty());
    }
}
