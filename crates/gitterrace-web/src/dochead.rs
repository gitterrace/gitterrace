//! The one header of a document card: the README card of the overview and of a folder, and a
//! Markdown file's page. It holds the title (or the tab row of the about documents), the
//! Preview/Code switch where both exist, the "On this page" button and the actions. Every card
//! that shows a document holds a [`DocHead`], so no template writes a card header by hand.

use std::fmt;

use askama::filters::HtmlSafe;

use crate::fmt::esc;
use crate::seg::{Choice, Seg};

/// A tab of the about documents (README, LICENSE and so on).
#[derive(Debug)]
pub(crate) struct DocTab {
    pub(crate) href: String,
    pub(crate) label: &'static str,
    pub(crate) on: bool,
}

/// A link at the right end of the header.
#[derive(Debug)]
pub(crate) struct DocAction {
    pub(crate) label: &'static str,
    pub(crate) href: String,
}

/// A document card's header, as shown.
#[derive(Debug, Default)]
pub(crate) struct DocHead {
    /// Shown when there are no tabs.
    pub(crate) title: String,
    /// The about documents; empty when one is present.
    pub(crate) tabs: Vec<DocTab>,
    /// Whether the Preview/Code switch shows: rendered Markdown with its source.
    pub(crate) views: bool,
    /// The outline menu (`markdown::outline_menu`, icon only), empty below three headings.
    pub(crate) outline: String,
    pub(crate) actions: Vec<DocAction>,
}

impl fmt::Display for DocHead {
    /// The only place that writes a document card's header bar.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<div class=\"dochead sticky\" data-dochead>")?;
        if self.tabs.is_empty() {
            write!(f, "<span class=\"doc-title\">{}</span>", esc(&self.title))?;
        } else {
            f.write_str("<nav class=\"tabs doctabs\">")?;
            for t in &self.tabs {
                let on = if t.on {
                    " class=\"on\" aria-current=\"page\""
                } else {
                    ""
                };
                write!(f, "<a href=\"{}\"{on}>{}</a>", esc(&t.href), esc(t.label))?;
            }
            f.write_str("</nav>")?;
        }
        if self.views {
            let views = Seg {
                name: "View".to_owned(),
                buttons: true,
                choices: vec![
                    Choice::new("Preview", "rendered", true),
                    Choice::new("Code", "source", false),
                ],
                ..Seg::default()
            };
            write!(f, "{views}")?;
        }
        f.write_str("<span class=\"doc-tools\">")?;
        for a in &self.actions {
            write!(f, "<a href=\"{}\">{}</a>", esc(&a.href), esc(a.label))?;
        }
        // The outline menu is the crate's own escaped markup.
        f.write_str(&self.outline)?;
        f.write_str("</span></div>")
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
                DocTab {
                    href: "/?doc=a&b".into(),
                    label: "README",
                    on: true,
                },
                DocTab {
                    href: "/x".into(),
                    label: "LICENSE",
                    on: false,
                },
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
        let h = DocHead {
            title: "<x>".into(),
            views: true,
            ..DocHead::default()
        }
        .to_string();
        assert!(h.contains("<span class=\"doc-title\">&lt;x&gt;</span>"));
        assert!(h.contains("data-show=\"source\""));
    }
}
