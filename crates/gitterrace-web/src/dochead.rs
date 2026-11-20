//! The one header of a document card: the README card of the overview and of a folder, and a
//! Markdown file's page. It holds the title (or the tab row of the about documents), the
//! Preview/Code switch where both exist, the "On this page" button and the actions. Every card
//! that shows a document holds a [`DocHead`], so no template writes a card header by hand.

use std::fmt;

use askama::Template;
use askama::filters::HtmlSafe;

use crate::seg::{Choice, Seg};

/// A tab of the about documents (README, LICENSE and so on).
#[derive(Debug)]
pub(crate) struct DocTab {
    pub(crate) href: String,
    pub(crate) label: &'static str,
    pub(crate) on: bool,
}

#[derive(Debug)]
pub(crate) struct DocAction {
    pub(crate) label: &'static str,
    pub(crate) href: String,
}

/// One step of a path shown in place of the title: a folder or file name with its page.
#[derive(Debug)]
pub(crate) struct DocCrumb {
    pub(crate) name: String,
    pub(crate) href: String,
}

#[derive(Debug, Default)]
pub(crate) struct DocHead {
    /// Shown when there are no tabs.
    pub(crate) title: String,
    /// A file's path in place of the title, so the card's header is the page's only head row.
    pub(crate) crumbs: Vec<DocCrumb>,
    /// Muted facts before the actions (a file's size).
    pub(crate) meta: String,
    pub(crate) tabs: Vec<DocTab>,
    pub(crate) views: bool,
    /// The outline menu (`markdown::outline_menu`, icon only), empty below three headings.
    pub(crate) outline: String,
    pub(crate) actions: Vec<DocAction>,
    /// A switch of the card's own views in place of Preview/Code (the settings page's
    /// Form/Plain), with a control of links before the title (its file switch).
    pub(crate) switch: Option<(Seg, Seg)>,
    /// The crate's own escaped markup at the right end, after the actions (a Save button).
    pub(crate) tools: String,
}

/// The markup, from `dochead.html`: the only place that writes a document card's header bar.
/// `escape = "svg"` picks the crate's `& < > "` escaper, the bytes `fmt::esc` wrote here before.
#[derive(Template)]
#[template(path = "dochead.html", escape = "svg")]
struct DocHeadMarkup<'a> {
    h: &'a DocHead,
    /// Preview/Code, when the card shows it and has no switch of its own.
    views: Option<Seg>,
}

impl fmt::Display for DocHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let views = (self.switch.is_none() && self.views).then(|| Seg {
            name: "View".to_owned(),
            panes: "view".to_owned(),
            choices: vec![
                Choice::new("Preview", "rendered", true),
                Choice::new("Code", "source", false),
            ],
            ..Seg::default()
        });
        DocHeadMarkup { h: self, views }.render_into(f).map_err(|_| fmt::Error)
    }
}

impl HtmlSafe for DocHead {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_marks_the_active_tab() {
        let h = DocHead {
            tabs: vec![
                DocTab { href: "/?doc=a&b".into(), label: "README", on: true },
                DocTab { href: "/x".into(), label: "LICENSE", on: false },
            ],
            ..DocHead::default()
        }
        .to_string();
        assert!(h.starts_with("<div class=\"dochead sticky\" data-dochead>"));
        assert!(h.contains("href=\"/?doc=a&amp;b\" class=\"on\" aria-current=\"page\">README"));
        assert!(!h.contains("data-show"));
    }

    #[test]
    fn a_title_alone_when_there_are_no_tabs() {
        let h = DocHead { title: "<x>".into(), views: true, ..DocHead::default() }.to_string();
        assert!(h.contains("<span class=\"doc-title\">&lt;x&gt;</span>"));
        assert!(h.contains("data-show=\"source\""));
    }

    #[test]
    fn a_card_switch_replaces_preview_and_code() {
        let views = Seg {
            name: "Editor".into(),
            buttons: true,
            choices: vec![Choice::new("Form", "form", true)],
            ..Seg::default()
        };
        let h = DocHead {
            title: "~/x".into(),
            switch: Some((Seg::links("File", vec![]), views)),
            views: true,
            tools: "<button>Save</button>".into(),
            ..DocHead::default()
        }
        .to_string();
        assert!(h.contains("data-show=\"form\"") && !h.contains("data-show=\"source\""));
        assert!(h.ends_with("<button>Save</button></span></div>"));
    }
}
