//! The one segmented control: the period on every insights page, the contributors metric, the
//! languages axis and scale, the languages range presets and a document's Preview/Code switch.
//! Markup in `seg_control.html`. One container holds the choices, the one in force is filled and
//! carries `aria-current`, and a control whose choices do not fit one row becomes a native select,
//! by one rule for all. No template or other module writes the markup by hand, so the controls keep
//! one size, spacing and focus.

use std::fmt;

use askama::Template;
use askama::filters::HtmlSafe;

/// One choice of a [`Seg`]: a link to the page with that choice, or a button for a switch that
/// stays on the page (the document views).
#[derive(Debug, Clone)]
pub(crate) struct Choice {
    pub(crate) label: String,
    /// The link target; for a button, the value of its `data-show`.
    pub(crate) target: String,
    pub(crate) on: bool,
}

impl Choice {
    pub(crate) fn new(label: impl Into<String>, target: impl Into<String>, on: bool) -> Self {
        Self { label: label.into(), target: target.into(), on }
    }
}

/// A segmented control, as shown.
#[derive(Debug, Clone, Default)]
pub(crate) struct Seg {
    /// Visible caption before the choices and the group's accessible name. Empty hides the
    /// caption but `name` still names the group.
    pub(crate) caption: String,
    /// Accessible name of the group.
    pub(crate) name: String,
    pub(crate) choices: Vec<Choice>,
    /// Buttons that switch panes in place instead of links that load a page.
    pub(crate) buttons: bool,
    /// Radio group name. Set, the choices are radios and CSS shows the checked pane, no script.
    pub(crate) panes: String,
}

impl Seg {
    /// A control of links named `name`, with `name` as its visible caption.
    pub(crate) fn links(name: &str, choices: Vec<Choice>) -> Self {
        Self { caption: name.to_owned(), name: name.to_owned(), choices, ..Self::default() }
    }

    /// Hides the caption; the group keeps its accessible name.
    #[must_use]
    pub(crate) fn no_caption(mut self) -> Self {
        self.caption.clear();
        self
    }
}

/// The markup, from `seg_control.html`: the only place that writes a segmented control.
#[derive(Template)]
#[template(path = "seg_control.html")]
struct SegMarkup<'a> {
    s: &'a Seg,
}

impl fmt::Display for Seg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SegMarkup { s: self }.render_into(f).map_err(|_| fmt::Error)
    }
}

impl HtmlSafe for Seg {}

#[cfg(test)]
mod tests {
    use super::*;

    fn period() -> Seg {
        Seg::links(
            "Period",
            vec![
                Choice::new("1w", "/stats?period=1w", false),
                Choice::new("1m", "/stats?period=1m&a=\"b\"", true),
            ],
        )
    }

    #[test]
    fn the_choice_in_force_is_marked_once() {
        let s = period().to_string();
        assert_eq!(s.matches("aria-current=\"true\"").count(), 1);
        assert_eq!(s.matches(" selected").count(), 1);
        assert!(s.contains("role=\"group\" aria-label=\"Period\""));
    }

    #[test]
    fn targets_and_labels_are_escaped() {
        let s = period().to_string();
        assert!(s.contains("&amp;a=") || s.contains("&#38;a="));
        assert!(!s.contains("a=\"b\"") && !s.contains("&a="));
    }

    #[test]
    fn a_hidden_caption_keeps_the_group_name() {
        let s = period().no_caption().to_string();
        assert!(!s.contains("segcap"));
        assert!(s.contains("aria-label=\"Period\""));
    }

    #[test]
    fn buttons_switch_in_place_and_offer_the_same_select() {
        let s = Seg {
            name: "View".to_owned(),
            buttons: true,
            choices: vec![
                Choice::new("Preview", "rendered", true),
                Choice::new("Code", "source", false),
            ],
            ..Seg::default()
        }
        .to_string();
        assert!(s.contains("<button type=\"button\" data-show=\"rendered\" class=\"on\""));
        // Every control carries the select it becomes when its row does not fit.
        assert!(s.contains("<option value=\"source\">Code</option>"));
        assert!(!s.contains("<a "));
    }

    #[test]
    fn panes_are_radios_with_the_default_checked() {
        let s = Seg {
            name: "View".to_owned(),
            panes: "view".to_owned(),
            choices: vec![
                Choice::new("Preview", "rendered", true),
                Choice::new("Code", "source", false),
            ],
            ..Seg::default()
        }
        .to_string();
        assert!(s.contains("<input type=\"radio\" name=\"view\" value=\"rendered\" checked>"));
        assert!(s.contains("value=\"source\">Code</label>"));
        assert!(!s.contains("<button"));
    }
}
