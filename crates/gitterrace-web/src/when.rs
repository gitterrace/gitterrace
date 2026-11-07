//! The one date marker: relative text (or the day under `display.dates = "absolute"`) with
//! the full timestamp on hover and the ISO instant for machines. Every view model that
//! shows a date holds a [`When`], so a template cannot print a date without its tip.

use std::fmt;

use askama::filters::HtmlSafe;

use crate::fmt::{UNKNOWN_DATE, date, esc, iso};

/// A date as shown: `text` on the page, `tip` on hover or focus, `datetime` for the element.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct When {
    pub(crate) text: String,
    pub(crate) datetime: String,
    pub(crate) tip: String,
}

impl When {
    /// `time` seen in `offset` seconds east of UTC; `text` is already relative or absolute.
    /// An out-of-range time keeps its plain text and carries no machine date.
    pub(crate) fn new(text: String, time: i64, offset: i32) -> Self {
        let tip = date(time, offset);
        if tip == UNKNOWN_DATE {
            return Self {
                text,
                ..Self::default()
            };
        }
        let zone = if offset == 0 {
            "UTC".to_owned()
        } else {
            let a = offset.unsigned_abs();
            format!(
                "{}{:02}:{:02}",
                if offset < 0 { '-' } else { '+' },
                a / 3600,
                a % 3600 / 60
            )
        };
        let tip = format!("{tip} {zone}");
        Self {
            text,
            datetime: iso(time),
            tip,
        }
    }

    /// No date at all, such as a repository without commits.
    pub(crate) fn none() -> Self {
        Self::default()
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

impl fmt::Display for When {
    /// The only place on the server that writes a date's `<time>` element.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.datetime.is_empty() {
            return f.write_str(&esc(&self.text));
        }
        write!(
            f,
            "<time data-tip=\"{}\" datetime=\"{}\" tabindex=\"0\">{}</time>",
            esc(&self.tip),
            esc(&self.datetime),
            esc(&self.text)
        )
    }
}

impl HtmlSafe for When {}

#[cfg(test)]
mod tests {
    use super::When;
    use crate::fmt::ago;

    #[test]
    fn renders_one_time_element() {
        let w = ago(0, 86_400 * 3);
        assert_eq!(
            w.to_string(),
            "<time data-tip=\"1970-01-01 00:00 \
            UTC\" datetime=\"1970-01-01T00:00:00Z\" tabindex=\"0\">3 days ago</time>"
        );
    }

    #[test]
    fn unknown_and_empty_stay_plain() {
        assert_eq!(ago(i64::MIN, 0).to_string(), "unknown date");
        assert_eq!(When::none().to_string(), "");
        assert!(When::none().is_empty());
    }

    #[test]
    fn text_is_escaped() {
        let w = When::new("<b>".to_owned(), 0, 3600);
        assert!(w.to_string().contains("&lt;b&gt;"));
        assert_eq!(w.tip, "1970-01-01 01:00 +01:00");
    }
}
