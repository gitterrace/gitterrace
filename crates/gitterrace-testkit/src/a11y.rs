//! Static checks of a rendered page for what a screen reader needs from its markup.
//!
//! Each check holds for every page by construction: the tests that visit pages run them on
//! every body they get, so a new page or a new table is checked without being listed.

/// What `body` is missing, one line per problem; empty when the page passes.
#[must_use]
pub fn problems(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(tables(body));
    out
}

/// Every table has a header cell, each header cell the app writes says what it heads
/// (`scope`), and every table the app writes has a caption. A Markdown table (its `data-line`
/// marks the source lines) is the author's: the parser gives it header cells in a `thead`,
/// which need no scope, and it has no caption to give.
fn tables(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, table) in elements(body, "table").enumerate() {
        let open = &table[..table.find('>').unwrap_or(table.len())];
        let authored = open.contains(" data-line=\"");
        let name = format!("table {} ({})", i + 1, open.trim_start_matches("<table"));
        let heads: Vec<&str> = elements(table, "th").collect();
        if heads.is_empty() {
            out.push(format!("{name} has no header cell"));
        }
        if !authored && !table.contains("<caption") {
            out.push(format!("{name} has no caption"));
        }
        if !authored {
            let bare = heads
                .iter()
                .filter(|th| !th[..th.find('>').unwrap_or(th.len())].contains(" scope=\""))
                .count();
            if bare > 0 {
                out.push(format!("{name} has {bare} header cells without a scope"));
            }
        }
    }
    out
}

/// Each `<tag ...>...</tag>` in `html`, outermost first, nesting not followed: the app writes
/// no table inside a table.
fn elements<'a>(html: &'a str, tag: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut rest = html;
    std::iter::from_fn(move || {
        loop {
            let at = rest.find(&open)?;
            let after = rest[at + open.len()..].chars().next()?;
            if after != ' ' && after != '>' {
                rest = &rest[at + open.len()..];
                continue;
            }
            let end = rest[at..]
                .find(&close)
                .map_or(rest.len(), |e| at + e + close.len());
            let found = &rest[at..end];
            rest = &rest[end..];
            return Some(found);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::problems;

    #[test]
    fn a_table_without_header_cells_is_flagged() {
        let p =
            problems("<table class=\"entries\"><caption>x</caption><tr><td>a</td></tr></table>");
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("no header cell"), "{p:?}");
        // A thead alone holds no header cell.
        let p = problems("<table><caption>x</caption><thead><tr><td>a</td></tr></thead></table>");
        assert!(p.iter().any(|m| m.contains("no header cell")), "{p:?}");
    }

    #[test]
    fn an_app_table_needs_a_caption_and_scoped_headers() {
        let p = problems("<table><tr><th>a</th></tr><tr><td>1</td></tr></table>");
        assert!(p.iter().any(|m| m.contains("no caption")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("without a scope")), "{p:?}");
    }

    #[test]
    fn a_headed_table_passes_and_a_markdown_table_needs_only_headers() {
        let app = "<table><caption>x</caption><tr><th scope=\"col\">a</th></tr></table>";
        assert!(problems(app).is_empty(), "{:?}", problems(app));
        let md = "<table data-line=\"1-3\"><thead><tr><th>a</th></tr></thead></table>";
        assert!(problems(md).is_empty(), "{:?}", problems(md));
    }

    #[test]
    fn a_thead_tag_is_not_taken_for_a_header_cell_with_no_scope() {
        let t =
            "<table><caption>x</caption><thead><tr><th scope=\"col\">a</th></tr></thead></table>";
        assert!(problems(t).is_empty(), "{:?}", problems(t));
    }
}
