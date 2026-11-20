//! Static checks of a rendered page for what a screen reader needs from its markup.
//!
//! Each check holds for every page by construction: the tests that visit pages run them on
//! every body they get, so a new page or a new table is checked without being listed.

/// What `body` is missing, one line per problem; empty when the page passes.
#[must_use]
pub fn problems(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(top_heading(body));
    out.extend(tables(body));
    out.extend(diff_lines(body));
    out.extend(blame_runs(body));
    out
}

/// Every run of a blame page says its age in words: the heat bar beside it says it only in
/// colour. A run is the gutter cell that opens a commit's lines (`<div class="bl">`).
fn blame_runs(body: &str) -> Option<String> {
    let runs = body.matches("<div class=\"bl\">").count();
    let said = body
        .matches("<div class=\"bl\"><span class=\"vh\">Changed ")
        .count();
    (runs != said).then(|| format!("{} of {runs} blame runs do not say their age", runs - said))
}

/// Every changed line of a diff says in words whether it was added or removed: its colour and
/// its line numbers say it only to a sighted reader. Commit, compare, worktree and folded
/// diffs all come from one writer, so a line written another way is caught here.
fn diff_lines(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (cls, word) in [("add", "Added, "), ("del", "Removed, ")] {
        let open = format!("<div class=\"{cls}\"");
        let bare = body
            .match_indices(&open)
            .filter(|(i, _)| {
                let line = &body[*i..];
                let start = line.find('>').map_or(line.len(), |e| e + 1);
                !line[start..].starts_with(&format!("<span class=\"vh sign\">{word}</span>"))
            })
            .count();
        if bare > 0 {
            out.push(format!("{bare} diff lines ({cls}) do not say {word:?}"));
        }
    }
    out
}

/// A page has exactly one h1, so a screen reader's first heading is the page's name. A page
/// with none starts at its section headings, one with two leaves the reader to guess.
fn top_heading(body: &str) -> Option<String> {
    let n = elements(body, "h1").count();
    (n != 1).then(|| format!("the page has {n} h1 headings, not one"))
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
    use super::{problems, tables};

    #[test]
    fn a_page_needs_exactly_one_h1() {
        assert!(problems("<main><h2>x</h2></main>")[0].contains("0 h1"));
        let two = "<h1>Page</h1><div class=\"markdown-body\"><h1>Doc</h1></div>";
        assert!(problems(two)[0].contains("2 h1"));
        assert_eq!(
            problems("<h1 class=\"vh\">Page</h1><hr><header></header>"),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn a_blame_run_must_say_its_age() {
        let run = |words: &str| {
            format!("<h1>x</h1><td class=\"who h2\"><div class=\"bl\">{words}a</div></td>")
        };
        assert!(problems(&run(""))[0].contains("1 of 1 blame runs"));
        let said = run("<span class=\"vh\">Changed 1 to 3 years ago. </span>");
        assert!(problems(&said).is_empty(), "{:?}", problems(&said));
    }

    #[test]
    fn a_changed_diff_line_must_say_what_changed() {
        let page = |line: &str| format!("<h1>x</h1><div class=\"diff\">{line}</div>");
        let bare = page("<div class=\"add\" data-o=\"\" data-n=\"1\">a</div>");
        assert!(
            problems(&bare)[0].contains("Added"),
            "{:?}",
            problems(&bare)
        );
        let bare = page("<div class=\"del\" data-o=\"1\" data-n=\"\">a</div>");
        assert!(
            problems(&bare)[0].contains("Removed"),
            "{:?}",
            problems(&bare)
        );
        let said = page(
            "<div class=\"add\" data-o=\"\" data-n=\"1\">\
             <span class=\"vh sign\">Added, </span>a</div>\
             <div class=\"ctx\" data-o=\"1\" data-n=\"2\">b</div>",
        );
        assert!(problems(&said).is_empty(), "{:?}", problems(&said));
    }

    #[test]
    fn a_table_without_header_cells_is_flagged() {
        let p = tables("<table class=\"entries\"><caption>x</caption><tr><td>a</td></tr></table>");
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("no header cell"), "{p:?}");
        // A thead alone holds no header cell.
        let p = tables("<table><caption>x</caption><thead><tr><td>a</td></tr></thead></table>");
        assert!(p.iter().any(|m| m.contains("no header cell")), "{p:?}");
    }

    #[test]
    fn an_app_table_needs_a_caption_and_scoped_headers() {
        let p = tables("<table><tr><th>a</th></tr><tr><td>1</td></tr></table>");
        assert!(p.iter().any(|m| m.contains("no caption")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("without a scope")), "{p:?}");
    }

    #[test]
    fn a_headed_table_passes_and_a_markdown_table_needs_only_headers() {
        let app = "<table><caption>x</caption><tr><th scope=\"col\">a</th></tr></table>";
        assert!(tables(app).is_empty(), "{:?}", tables(app));
        let md = "<table data-line=\"1-3\"><thead><tr><th>a</th></tr></thead></table>";
        assert!(tables(md).is_empty(), "{:?}", tables(md));
    }

    #[test]
    fn a_thead_tag_is_not_taken_for_a_header_cell_with_no_scope() {
        let t =
            "<table><caption>x</caption><thead><tr><th scope=\"col\">a</th></tr></thead></table>";
        assert!(tables(t).is_empty(), "{:?}", tables(t));
    }
}
