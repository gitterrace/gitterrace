//! Markdown to HTML for READMEs. Raw HTML is shown as text, except comments, which are dropped.

use pulldown_cmark::{Event, Options, Parser, html};

/// Removes a leading `---` YAML block so it is not rendered as a setext heading.
fn strip_front_matter(src: &str) -> &str {
    let Some(rest) = src
        .strip_prefix("---\n")
        .or_else(|| src.strip_prefix("---\r\n"))
    else {
        return src;
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end() == "---" {
            return &rest[offset..];
        }
    }
    src
}

pub(crate) fn render(src: &str) -> String {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(strip_front_matter(src), opts).filter_map(|e| match e {
        Event::Html(t) | Event::InlineHtml(t) if t.trim_start().starts_with("<!--") => None,
        Event::Html(t) | Event::InlineHtml(t) => Some(Event::Text(t)),
        other => Some(other),
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

    #[test]
    fn drops_front_matter_and_comments() {
        let out = render("---\ntitle: x\n---\n\n<!-- note -->\n\n# Real\n");
        assert!(!out.contains("title"));
        assert!(!out.contains("note"));
        assert!(out.contains("<h1>Real</h1>"));
    }
}
