//! Making an exported page work from disk: a `file://` URL has no site root, so a link written
//! as `/static/app.css` points at the filesystem root and the page opens blank. Every
//! same-origin absolute link is rewritten relative to the page that carries it, and a link to
//! a page names its `index.html`, because a file URL does not serve a folder's index.

/// Paths that are files as exported, not pages: they take no `index.html`.
const FILE_PREFIXES: [&str; 3] = ["/static/", "/raw/", "/snapshot.css"];

/// `body` of the page exported at `path`, with its root-relative links made relative.
/// `is_page` is true for HTML (written as `<path>/index.html`), false for a stylesheet.
pub(super) fn relativize(body: &str, path: &str, is_page: bool) -> String {
    let segments = path.split('/').filter(|s| !s.is_empty()).count();
    let ups = if is_page {
        segments
    } else {
        segments.saturating_sub(1)
    };
    let up = "../".repeat(ups);
    let mut out = String::with_capacity(body.len() + body.len() / 8);
    let mut rest = body;
    while let Some((at, marker)) = next_marker(rest) {
        let value = at + marker.len();
        out.push_str(&rest[..value]);
        rest = &rest[value..];
        // `url('x')` opens with a quote of its own.
        let quote = rest.len() - rest.trim_start_matches(['"', '\'']).len();
        out.push_str(&rest[..quote]);
        rest = &rest[quote..];
        let end = rest.find(['"', ')', '\'']).unwrap_or(rest.len());
        let raw = &rest[..end];
        if raw.starts_with('/') && !raw.starts_with("//") {
            out.push_str(&rewrite(raw, &up));
        } else {
            out.push_str(raw);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn next_marker(s: &str) -> Option<(usize, &'static str)> {
    ["href=\"", "src=\"", "url("]
        .into_iter()
        .filter_map(|m| s.find(m).map(|i| (i, m)))
        .min_by_key(|(i, _)| *i)
}

fn rewrite(raw: &str, up: &str) -> String {
    let cut = raw.find(['?', '#']).unwrap_or(raw.len());
    let (path, tail) = raw.split_at(cut);
    let is_file = FILE_PREFIXES.iter().any(|p| path.starts_with(p));
    let local = path.trim_start_matches('/');
    let rel = if is_file {
        format!("{up}{local}")
    } else if local.is_empty() {
        format!("{up}index.html")
    } else {
        format!("{up}{}/index.html", local.trim_end_matches('/'))
    };
    // A page at the root with nothing to climb still needs a path that is not a bare name
    // starting with a colon-bearing segment: `./` keeps it a relative path.
    let rel = if up.is_empty() {
        format!("./{rel}")
    } else {
        rel
    };
    format!("{rel}{tail}")
}

#[cfg(test)]
mod tests {
    use super::relativize;

    #[test]
    fn pages_and_assets_link_relative_to_the_page() {
        let html = concat!(
            r#"<link href="/static/app.css"><a href="/blob/HEAD/a.md#L2">a</a>"#,
            r#"<a href="/">h</a><a href="https://x.dev/">e</a><img src="/raw/HEAD/a.png">"#
        );
        assert_eq!(
            relativize(html, "/tree/HEAD", true),
            concat!(
                r#"<link href="../../static/app.css">"#,
                r#"<a href="../../blob/HEAD/a.md/index.html#L2">a</a>"#,
                r#"<a href="../../index.html">h</a><a href="https://x.dev/">e</a>"#,
                r#"<img src="../../raw/HEAD/a.png">"#
            )
        );
        assert_eq!(
            relativize(r#"<a href="/refs">r</a>"#, "/", true),
            r#"<a href="./refs/index.html">r</a>"#
        );
    }

    #[test]
    fn stylesheet_urls_are_relative_to_the_stylesheet() {
        assert_eq!(
            relativize(
                "b{background:url('/static/vendor/f.woff2')}",
                "/static/app.css",
                false
            ),
            "b{background:url('../static/vendor/f.woff2')}"
        );
    }
}
