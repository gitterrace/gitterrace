//! Making an exported page work from disk: a `file://` URL has no site root, so a link written
//! as `/static/app.css` points at the filesystem root and the page opens blank. Every
//! same-origin absolute link is rewritten relative to the page that carries it, and a link to
//! a page names its `index.html`, because a file URL does not serve a folder's index.

use std::ops::Range;

/// Paths that are files as exported, not pages: they take no `index.html`.
const FILE_PREFIXES: [&str; 3] = ["/static/", "/raw/", "/snapshot.css"];

/// `body` of the page exported at `path`, with its root-relative links made relative.
/// `is_page` is true for HTML (written as `<path>/index.html`), false for a stylesheet.
pub(super) fn relativize(body: &str, path: &str, is_page: bool) -> String {
    let segments = path.split('/').filter(|s| !s.is_empty()).count();
    let ups = if is_page { segments } else { segments.saturating_sub(1) };
    let up = "../".repeat(ups);
    let mut out = String::with_capacity(body.len() + body.len() / 8);
    let mut done = 0;
    for span in link_spans(body, &MARKERS) {
        out.push_str(&body[done..span.start]);
        let raw = &body[span.clone()];
        if is_site_path(raw) {
            out.push_str(&rewrite(raw, &up));
        } else {
            out.push_str(raw);
        }
        done = span.end;
    }
    out.push_str(&body[done..]);
    out
}

/// What opens a link value in a rendered page.
pub(super) const MARKERS: [&str; 3] = ["href=\"", "src=\"", "url("];

/// The value after each of `markers`, in page order, without its quotes: the byte range of
/// every `href="..."`, `src="..."` and `url(...)` target.
pub(super) fn link_spans<'a>(
    html: &'a str,
    markers: &'a [&'static str],
) -> impl Iterator<Item = Range<usize>> + 'a {
    let mut pos = 0;
    std::iter::from_fn(move || {
        let rest = &html[pos..];
        let (at, marker) =
            markers.iter().filter_map(|m| rest.find(m).map(|i| (i, *m))).min_by_key(|(i, _)| *i)?;
        let value = pos + at + marker.len();
        // `url('x')` opens with a quote of its own.
        let quoted = &html[value..];
        let start = value + quoted.len() - quoted.trim_start_matches(['"', '\'']).len();
        let end = html[start..].find(['"', ')', '\'']).map_or(html.len(), |e| start + e);
        pos = end;
        Some(start..end)
    })
}

/// A link to this site: rooted, and not a `//host` one.
pub(super) fn is_site_path(raw: &str) -> bool {
    raw.starts_with('/') && !raw.starts_with("//")
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
    let rel = if up.is_empty() { format!("./{rel}") } else { rel };
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
            relativize("b{background:url('/static/vendor/f.woff2')}", "/static/app.css", false),
            "b{background:url('../static/vendor/f.woff2')}"
        );
    }
}
