//! The one empty state: a page or section with nothing to show says what is missing and, where
//! there is one, offers the next step as a link. Markup in `empty_state.html`. No template or
//! other module writes the markup by hand, so every empty list reads and spaces the same.

use askama::Template;
use askama::filters::HtmlSafe;

#[derive(Debug, Clone, Default, Template)]
#[template(path = "empty_state.html")]
pub(crate) struct Empty {
    pub(crate) message: String,
    /// The next step: its label and link target. `None` when nothing leads anywhere better.
    pub(crate) step: Option<(String, String)>,
}

impl Empty {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), step: None }
    }

    /// Offers `label` as a link to `href` under the message.
    #[must_use]
    pub(crate) fn step(mut self, label: impl Into<String>, href: impl Into<String>) -> Self {
        self.step = Some((label.into(), href.into()));
        self
    }
}

impl HtmlSafe for Empty {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_step_is_a_link_under_the_message() {
        let s = Empty::new("No stashes.").step("Branches & tags", "/refs").to_string();
        assert!(s.starts_with("<div class=\"empty\""), "{s}");
        assert!(s.contains("<p>No stashes.</p>"), "{s}");
        assert!(s.contains("<p class=\"next\"><a href=\"/refs\">Branches &"), "{s}");
    }

    #[test]
    fn without_a_step_only_the_message_shows() {
        let s = Empty::new("No tags.").to_string();
        assert!(!s.contains("<a "), "{s}");
    }

    #[test]
    fn message_and_target_are_escaped() {
        let s = Empty::new("<b>").step("x", "/a?b=\"c\"").to_string();
        assert!(!s.contains("<b>") && !s.contains("b=\"c\""), "{s}");
    }
}
