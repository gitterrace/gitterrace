//! Lines added and removed, `+a -d` in their colours, for pages built in Rust. Templates use the
//! `_delta.html` macro, which writes the same markup.

/// The markup for `add` lines added and `del` removed, each grouped in thousands.
pub(crate) fn line_delta(add: impl std::fmt::Display, del: impl std::fmt::Display) -> String {
    format!(
        "<b class=\"add\">+{}</b> <b class=\"del\">-{}</b>",
        crate::fmt::thousands(add),
        crate::fmt::thousands(del)
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_both_counts_grouped() {
        assert_eq!(
            super::line_delta(1200, 3),
            "<b class=\"add\">+1,200</b> <b class=\"del\">-3</b>"
        );
    }
}
