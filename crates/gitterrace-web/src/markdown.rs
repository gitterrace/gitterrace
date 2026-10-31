//! Markdown to HTML for READMEs. Raw HTML in the source is shown as text, not injected.

use pulldown_cmark::{Event, Options, Parser, html};

pub(crate) fn render(src: &str) -> String {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(src, opts).map(|e| match e {
        Event::Html(t) | Event::InlineHtml(t) => Event::Text(t),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

#[cfg(test)]
mod tests {
    use crate::markdown::render;

    #[test]
    fn renders_and_neutralizes_html() {
        let out = render("# Hi\n\n<script>alert(1)</script>\n");
        assert!(out.contains("<h1>Hi</h1>"));
        assert!(!out.contains("<script>"));
    }
}
