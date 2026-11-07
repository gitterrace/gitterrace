//! Heading levels of a rendered document, one below the page's.
//!
//! The page head owns the page's one h1, so a document's own headings move down a level: its
//! `#` is an h2, its `##` an h3, and so on. An h6 stays an h6, as HTML has no h7. Each heading
//! keeps a `markdown-hN` class with its source level, so it looks as the author wrote it.

/// `html` with every heading one level lower, its opening tag marked with the source level.
/// Runs on sanitized markup, so every heading tag is a plain `<hN` or `</hN>`.
pub(super) fn demote(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + html.len() / 16);
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        let (head, tail) = rest.split_at(i);
        out.push_str(head);
        let close = tail.starts_with("</");
        let at = if close { 2 } else { 1 };
        let b = tail.as_bytes();
        if let (Some(b'h'), Some(&d @ b'1'..=b'6'), Some(b' ' | b'>')) =
            (b.get(at), b.get(at + 1), b.get(at + 2))
        {
            out.push_str(&tail[..=at]);
            out.push(char::from((d + 1).min(b'6')));
            if !close {
                out.push_str(" class=\"markdown-h");
                out.push(char::from(d));
                out.push('"');
            }
            rest = &tail[at + 2..];
        } else {
            out.push('<');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::demote;

    #[test]
    fn every_heading_moves_down_one_level_and_h6_stays() {
        let html = "<h1 id=\"a\">A</h1><h2>B</h2><h5>E</h5><h6>F</h6>";
        assert_eq!(
            demote(html),
            "<h2 class=\"markdown-h1\" id=\"a\">A</h2><h3 class=\"markdown-h2\">B</h3>\
             <h6 class=\"markdown-h5\">E</h6><h6 class=\"markdown-h6\">F</h6>"
        );
    }

    #[test]
    fn other_tags_that_start_with_h_are_kept() {
        let html = "<hr><header>x</header><p>h1</p>";
        assert_eq!(demote(html), html);
    }
}
