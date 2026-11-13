//! The docs navigation from `templates/docnav.html`: the list of every docs page beside the
//! text, the same list folded into the bar's Docs menu on a narrow screen, the page's outline
//! folded into an "On this page" menu above the text, and the previous and next links at the
//! end of a page. One order, the docs index's, for all of them.

use askama::Template;

use crate::site::bar::DOCS_ICON;
use crate::site::frame::pack;

/// One docs page in the list. `current` is the attribute that marks the page shown, fixed
/// markup or empty.
#[derive(Clone)]
struct Item<'a> {
    href: &'a str,
    title: &'a str,
    current: &'static str,
    /// The group heading shown above this page, set only on the first page of its group.
    group: &'a str,
}

#[derive(Template)]
#[template(path = "docnav.html")]
struct Nav<'a> {
    part: &'a str,
    /// The Docs icon on the menu's summary, as the bar's Docs link draws it.
    docs_icon: &'a str,
    docs: Vec<Item<'a>>,
    heads: &'a [(String, String)],
    /// (site path, title), empty for none.
    prev: (&'a str, &'a str),
    next: (&'a str, &'a str),
}

/// The navigation of the docs page at `here` among `order`, every docs page as (site path,
/// title, group heading) in index order. `heads` are the page's level 2 headings as (id, text)
/// for the narrow "On this page" menu, empty when the page has no outline.
pub(super) struct DocNav {
    /// The list beside the text on a wide screen.
    pub list: String,
    /// The bar's Docs menu, the list on a narrow screen, in place of the bar's Docs link.
    pub menu: String,
    /// The "On this page" menu above the text on a narrow screen, empty without an outline.
    pub menus: String,
    /// Previous and next under the text.
    pub pager: String,
}

pub(super) fn nav(
    order: &[(String, String, String)],
    here: &str,
    heads: &[(String, String)],
) -> DocNav {
    let docs: Vec<Item> = order
        .iter()
        .enumerate()
        .map(|(i, (p, t, g))| Item {
            group: if i == 0 || order[i - 1].2 != *g {
                g
            } else {
                ""
            },
            href: p,
            title: t,
            current: if p == here {
                " aria-current=\"page\""
            } else {
                ""
            },
        })
        .collect();
    let at = order.iter().position(|(p, ..)| p == here);
    let pair = |i: usize| order.get(i).map(|(p, t, _)| (p.as_str(), t.as_str()));
    let prev = at.and_then(|i| i.checked_sub(1)).and_then(pair);
    let next = at.and_then(|i| pair(i + 1));
    let render = |part: &str| {
        let html = Nav {
            part,
            docs_icon: DOCS_ICON,
            docs: docs.clone(),
            heads,
            prev: prev.unwrap_or(("", "")),
            next: next.unwrap_or(("", "")),
        }
        .render()
        .unwrap_or_default();
        if html.is_empty() {
            return html;
        }
        let mut out = pack(&html);
        out.push('\n');
        out
    };
    DocNav {
        list: render("list"),
        menu: render("menu"),
        menus: render("outline"),
        pager: render("pager"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> Vec<(String, String, String)> {
        ["a", "b", "c"]
            .iter()
            .map(|n| (format!("docs/{n}/"), n.to_uppercase(), "G".to_owned()))
            .collect()
    }

    #[test]
    fn the_current_page_is_marked_and_its_neighbours_are_previous_and_next() {
        let n = nav(&order(), "docs/b/", &[]);
        assert_eq!(n.list.matches("aria-current=\"page\"").count(), 1);
        assert!(
            n.list
                .contains("<a href=\"/docs/b/\" aria-current=\"page\">B</a>")
        );
        assert!(
            n.pager.contains("href=\"/docs/a/\" rel=\"prev\""),
            "{}",
            n.pager
        );
        assert!(
            n.pager.contains("href=\"/docs/c/\" rel=\"next\""),
            "{}",
            n.pager
        );
        assert!(n.menus.is_empty(), "{}", n.menus);
        assert!(
            n.menu
                .starts_with("<details class=\"menu docsmenu\"><summary>")
        );
        let summary = n.menu.split("</summary>").next().unwrap_or_default();
        assert!(
            summary.contains("<span>Docs</span>") && summary.contains("M12 6.5C10.5"),
            "the menu draws the Docs link's icon and word: {summary}"
        );
        assert_eq!(n.menu.matches("<li>").count(), 3, "{}", n.menu);
    }

    #[test]
    fn the_first_page_has_no_previous_and_the_last_no_next() {
        let first = nav(&order(), "docs/a/", &[]);
        assert!(!first.pager.contains("rel=\"prev\""), "{}", first.pager);
        let last = nav(&order(), "docs/c/", &[]);
        assert!(!last.pager.contains("rel=\"next\""), "{}", last.pager);
    }

    #[test]
    fn a_page_with_an_outline_folds_it_into_a_menu() {
        let heads = vec![("x".to_owned(), "X <y>".to_owned())];
        let n = nav(&order(), "docs/a/", &heads);
        assert!(
            n.menus.contains("<summary>On this page</summary>"),
            "{}",
            n.menus
        );
        assert!(
            n.menus.contains("href=\"#x\"") && !n.menus.contains("<y>"),
            "{}",
            n.menus
        );
        assert!(
            n.menus.lines().all(|l| l.chars().count() <= 100),
            "{}",
            n.menus
        );
    }
}
