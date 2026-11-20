//! Escaping for HTML and SVG text, written once so a page and a card cannot disagree.

/// Escapes element content only: quotes stay as they are, which keeps JSON-heavy sources small.
#[must_use]
pub fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Escapes text for an HTML or SVG attribute or element: `& < > "`.
#[must_use]
pub fn esc(s: &str) -> String {
    esc_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::{esc, esc_text};

    #[test]
    fn escapes_markup_and_quotes() {
        assert_eq!(esc("a&<>\"b"), "a&amp;&lt;&gt;&quot;b");
        assert_eq!(esc_text("\"x\""), "\"x\"");
    }
}
