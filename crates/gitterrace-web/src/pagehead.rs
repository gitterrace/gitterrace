//! The one page head. Every page under the top bar opens with it, and its slots come in one
//! order: an optional eyebrow, the title row (title, a muted qualifier, actions and tools), the
//! tagline, then either the tab row or a plain divider, then the toolbar. Markup in
//! `pagehead.html`, styles in the "Page head" block of `app.css`. No other template writes an
//! h1 or a page's head row, so every page starts at the same place with the same rhythm.

use std::fmt;

use askama::Template;
use askama::filters::HtmlSafe;

use crate::views::Crumb;

mod heads;

pub(crate) use heads::{
    FileView, file_head, insights_head, lead_tagline, overview_head, refs_head, waiting_head,
};

/// A link in the title row, right of the title: the page's main jump (History, Compare).
#[derive(Debug, Clone)]
pub(crate) struct Action {
    pub(crate) label: &'static str,
    pub(crate) href: String,
}

/// One tab of the tab row: a sub-view of the page with its own address.
#[derive(Debug, Clone)]
pub(crate) struct Tab {
    pub(crate) label: String,
    pub(crate) href: String,
    pub(crate) on: bool,
}

/// One run of the tagline: words, a number or a link.
#[derive(Debug, Clone)]
pub(crate) struct Part {
    pub(crate) text: String,
    pub(crate) num: bool,
    pub(crate) href: String,
}

/// The line under the title: one sentence with the page's key numbers, numbers set apart.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tagline {
    pub(crate) parts: Vec<Part>,
    /// The sentence is the insights finding that `leads_match_the_numbers` checks.
    pub(crate) lead: bool,
}

impl Tagline {
    /// `s` as the tagline, each number in it (digits with their separators and a trailing
    /// percent sign) set apart.
    pub(crate) fn of(s: &str) -> Self {
        let mut parts: Vec<Part> = Vec::new();
        let mut push = |text: &str, num: bool| {
            if text.is_empty() {
                return;
            }
            match parts.last_mut() {
                Some(p) if p.num == num && p.href.is_empty() => p.text.push_str(text),
                _ => parts.push(Part {
                    text: text.to_owned(),
                    num,
                    href: String::new(),
                }),
            }
        };
        let b = s.as_bytes();
        let mut i = 0;
        let mut start = 0;
        while i < b.len() {
            if b[i].is_ascii_digit() {
                push(&s[start..i], false);
                let from = i;
                while i < b.len()
                    && (b[i].is_ascii_digit()
                        || ((b[i] == b',' || b[i] == b'.')
                            && b.get(i + 1).is_some_and(u8::is_ascii_digit)))
                {
                    i += 1;
                }
                if b.get(i) == Some(&b'%') {
                    i += 1;
                }
                push(&s[from..i], true);
                start = i;
            } else {
                i += 1;
            }
        }
        push(&s[start..], false);
        Self { parts, lead: false }
    }

    /// Marks the tagline as the page's lead sentence.
    #[must_use]
    pub(crate) const fn lead(mut self) -> Self {
        self.lead = true;
        self
    }

    /// Appends `label` as a link to `href`, after `sep`.
    #[must_use]
    pub(crate) fn link(mut self, sep: &str, label: impl Into<String>, href: String) -> Self {
        if !sep.is_empty() {
            self.parts.push(Part {
                text: sep.to_owned(),
                num: false,
                href: String::new(),
            });
        }
        self.parts.push(Part {
            text: label.into(),
            num: false,
            href,
        });
        self
    }
}

/// A page's head, as shown. Only `title` (or `crumbs`) and `tagline` are always filled.
#[derive(Debug, Clone, Default)]
pub(crate) struct PageHead {
    /// The group the page belongs to, above the title. Empty for most pages.
    pub(crate) eyebrow: String,
    /// The page's name, its one h1. Ignored when `crumbs` are set.
    pub(crate) title: String,
    /// A path in place of the title, each part a link.
    pub(crate) crumbs: Vec<Crumb>,
    /// The page's scope after the title, muted and mono (`HEAD`, `tag`).
    pub(crate) qualifier: String,
    pub(crate) actions: Vec<Action>,
    /// The crate's own escaped markup at the title row's end, after the actions (an outline
    /// menu, a Save button).
    pub(crate) tools: String,
    pub(crate) tagline: Tagline,
    /// Sub-views of the page. Fewer than two shows the divider instead: never a one-tab row.
    pub(crate) tabs: Vec<Tab>,
    /// The crate's own escaped markup for the toolbar: segmented controls, a filter, facts.
    /// Empty for no toolbar.
    pub(crate) toolbar: String,
    /// The head stays at the top of the window while long content scrolls (wide screens).
    pub(crate) pin: bool,
}

impl PageHead {
    /// A head with `title` and `tagline`, every other slot empty.
    pub(crate) fn new(title: impl Into<String>, tagline: Tagline) -> Self {
        Self {
            title: title.into(),
            tagline,
            ..Self::default()
        }
    }
}

/// The markup, from `pagehead.html`: the only template that writes an h1.
#[derive(Template)]
#[template(path = "pagehead.html")]
struct PageHeadMarkup<'a> {
    h: &'a PageHead,
    tabs: bool,
}

impl fmt::Display for PageHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        PageHeadMarkup {
            h: self,
            tabs: self.tabs.len() > 1,
        }
        .render_into(f)
        .map_err(|_| fmt::Error)
    }
}

impl HtmlSafe for PageHead {}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(t: &Tagline) -> Vec<(String, bool)> {
        t.parts.iter().map(|p| (p.text.clone(), p.num)).collect()
    }

    #[test]
    fn numbers_in_the_tagline_are_set_apart() {
        let t = Tagline::of("1,234 commits by 2 authors, 48% in 3.5 days.");
        let nums: Vec<String> = t
            .parts
            .iter()
            .filter(|p| p.num)
            .map(|p| p.text.clone())
            .collect();
        assert_eq!(nums, ["1,234", "2", "48%", "3.5"]);
        assert_eq!(texts(&t).last().map(|p| p.0.as_str()), Some(" days."));
    }

    #[test]
    fn slots_come_in_one_order_with_the_divider_when_there_are_no_tabs() {
        let h = PageHead {
            eyebrow: "Group".into(),
            qualifier: "HEAD".into(),
            toolbar: "<span>t</span>".into(),
            ..PageHead::new("Commits", Tagline::of("3 commits"))
        }
        .to_string();
        let at = |s: &str| h.find(s).unwrap_or(usize::MAX);
        let order = [
            "class=\"ph-eyebrow\"",
            "<h1>Commits",
            "class=\"ph-tag\"",
            "class=\"ph-sep\"",
            "class=\"fbar\"",
        ]
        .map(at);
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?} in {h}");
        assert!(!h.contains("<nav class=\"tabs\">"));
    }

    #[test]
    fn one_tab_is_no_tab_row_and_two_are() {
        let tab = |label: &str, on| Tab {
            label: label.into(),
            href: format!("/{label}"),
            on,
        };
        let one = PageHead {
            tabs: vec![tab("A", true)],
            ..PageHead::new("T", Tagline::of("x"))
        }
        .to_string();
        assert!(!one.contains("class=\"tabs\"") && one.contains("ph-sep"));
        let two = PageHead {
            tabs: vec![tab("A", true), tab("B&C", false)],
            ..PageHead::new("T", Tagline::of("x"))
        }
        .to_string();
        assert!(two.contains("<nav class=\"tabs\">") && !two.contains("ph-sep"));
        assert!(two.contains("aria-current=\"page\">A<"));
        assert!(
            two.contains(">B&amp;C<") || two.contains(">B&#38;C<"),
            "{two}"
        );
    }

    #[test]
    fn a_path_title_links_each_part_and_escapes() {
        let h = PageHead {
            crumbs: vec![
                Crumb {
                    name: "main".into(),
                    href: "/tree/main".into(),
                },
                Crumb {
                    name: "<a>".into(),
                    href: "/tree/main/x".into(),
                },
            ],
            ..PageHead::new("ignored", Tagline::of("x"))
        }
        .to_string();
        assert!(h.contains("<h1 class=\"crumbs\">"), "{h}");
        assert!(!h.contains("<a></a>") && !h.contains(">ignored<"), "{h}");
        assert!(h.contains("&lt;a&gt;") || h.contains("&#60;a&#62;"), "{h}");
    }

    #[test]
    fn a_lead_tagline_carries_the_marker() {
        let h = PageHead::new("Insights", Tagline::of("3 commits.").lead()).to_string();
        assert!(h.contains("data-lead>"), "{h}");
    }
}
