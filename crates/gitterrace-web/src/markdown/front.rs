//! YAML-style front matter: a leading `---` block shown as a key/value table, or verbatim.

use std::fmt::Write as _;

use crate::fmt::esc;

/// Splits a leading `---` block from the document. Returns the block body and the rest.
/// An unterminated block is not front matter.
pub(super) fn split_front_matter(src: &str) -> Option<(&str, &str)> {
    let rest = src
        .strip_prefix("---\n")
        .or_else(|| src.strip_prefix("---\r\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

/// Parses the simple `key: value` and `key:` + `- item` forms. None when any line
/// is something else, so the caller shows the block verbatim.
fn parse_front_matter(block: &str) -> Option<Vec<(String, String)>> {
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    for line in block.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            rows.last_mut()?.1.push(unquote(item.trim())?);
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            return None;
        }
        let (key, value) = line.split_once(':')?;
        let key = key.trim();
        if key.is_empty()
            || key.contains(|c: char| c.is_whitespace() || "&*!|>{}[]#'\"".contains(c))
        {
            return None;
        }
        let value = value.trim();
        if value.is_empty() {
            rows.push((key.to_owned(), Vec::new()));
        } else {
            rows.push((key.to_owned(), vec![unquote(value)?]));
        }
    }
    if rows.is_empty() {
        return None;
    }
    Some(rows.into_iter().map(|(k, v)| (k, v.join(", "))).collect())
}

/// Plain, or wrapped in one matching pair of quotes. Anchors, aliases, block
/// scalars and flow collections are not handled.
fn unquote(v: &str) -> Option<String> {
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q) {
            return inner.strip_suffix(q).map(str::to_owned);
        }
    }
    if v.starts_with(['&', '*', '!', '|', '>', '{', '[', '#']) {
        return None;
    }
    Some(v.to_owned())
}

/// HTML for the front matter block: a key/value table, or the raw text as code.
/// Built here and added after sanitizing, so every piece is escaped by hand.
pub(super) fn front_matter_html(block: &str) -> String {
    parse_front_matter(block).map_or_else(
        || format!("<pre><code>{}</code></pre>\n", esc(block)),
        |rows| {
            let mut out = String::from("<table class=\"fm\">");
            for (k, v) in rows {
                let _ = write!(out, "<tr><th>{}</th><td>{}</td></tr>", esc(&k), esc(&v));
            }
            out.push_str("</table>\n");
            out
        },
    )
}
