//! Checks over the whole built site, as `site/` will hold it: pages, `_headers`, `_redirects`,
//! the 404 page, `sitemap.xml` and the stylesheet. Each check is a function over data, run on the
//! real output by one test and on a planted fault by another, so it is seen to go red.

use crate::site::tests::{generated, unresolved};
use crate::site::*;

/// The text of the built file at `path` (relative to `site/`).
fn text_of(path: &str) -> String {
    let root = crate::repo_root();
    let ws = crate::facts::load(&root).expect("facts");
    for out in render(&root, &ws).expect("render") {
        if let Out::File { path: p, body } = out
            && p.strip_prefix("site/") == Some(path)
        {
            return body;
        }
    }
    panic!("no built file {path}");
}

/// The site path a URL of this site names: the part after the host, without the leading `/`.
fn site_path(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |u| u.1);
    let path = rest.split_once('/')?.1;
    Some(path.split('#').next().unwrap_or("").to_owned())
}

/// Every sitemap URL, `_redirects` target on this site and stylesheet `url()` that names no
/// built file. `home` is the site's origin, so a redirect to another origin is not ours.
fn unresolved_host_refs(
    sitemap: &str,
    redirects: &str,
    css: &str,
    home: &str,
    files: &[String],
) -> Vec<String> {
    let known = |p: &str| {
        let p = if p.is_empty() || p.ends_with('/') {
            format!("{p}index.html")
        } else {
            p.to_owned()
        };
        files.contains(&p)
    };
    let mut bad = Vec::new();
    for loc in sitemap.split("<loc>").skip(1) {
        let url = loc.split("</loc>").next().unwrap_or("");
        match url.strip_prefix(home).and_then(|r| r.strip_prefix('/')) {
            Some(p) if known(p) => {}
            _ => bad.push(format!("sitemap.xml: {url}: no built file")),
        }
    }
    for line in redirects.lines().filter(|l| !l.starts_with('#')) {
        let mut parts = line.split_whitespace();
        let (Some(from), Some(to)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !from.starts_with('/') {
            bad.push(format!("_redirects: {from}: a source must start with /"));
        }
        let ours = to.starts_with('/') || to.starts_with(home);
        if ours && !site_path(to).is_some_and(|p| known(&p)) {
            bad.push(format!("_redirects: {from} -> {to}: no built file"));
        }
    }
    for url in css.split("url(").skip(1) {
        let dest = url
            .split(')')
            .next()
            .unwrap_or("")
            .trim_matches(['"', '\'']);
        if !dest.starts_with("data:") && !files.contains(&format!("assets/{dest}")) {
            bad.push(format!("assets/site.css: url({dest}): no built file"));
        }
    }
    bad
}

/// The class this guards: a link, sitemap entry, redirect target or stylesheet url that points at
/// a file the build did not write. Read off the output, not the sources that make it.
#[test]
fn every_built_reference_resolves_to_a_built_file() {
    let (pages, files) = generated();
    assert!(pages.iter().any(|p| p.0 == "404.html"), "404.html is built");
    let bad = unresolved(&pages, &files);
    assert!(bad.is_empty(), "unresolved:\n{}", bad.join("\n"));
    let home = site_path_home();
    let bad = unresolved_host_refs(
        &text_of("sitemap.xml"),
        &text_of("_redirects"),
        &text_of("assets/site.css"),
        &home,
        &files,
    );
    assert!(bad.is_empty(), "unresolved:\n{}", bad.join("\n"));
}

/// The site's origin as the sitemap writes it.
fn site_path_home() -> String {
    let sm = text_of("sitemap.xml");
    let first = sm
        .split("<loc>")
        .nth(1)
        .and_then(|l| l.split("</loc>").next());
    first
        .expect("a sitemap entry")
        .trim_end_matches('/')
        .to_owned()
}

#[test]
fn a_planted_dead_sitemap_url_redirect_or_font_is_reported() {
    let (_, files) = generated();
    let home = "https://h.example";
    let sitemap = "<loc>https://h.example/</loc><loc>https://h.example/docs/gone/</loc>";
    let redirects = "# c\n/a /nowhere.html 302\n/b https://elsewhere.example/x 302\n";
    let css = "@font-face{src:url(fonts/missing.woff2)}";
    let bad = unresolved_host_refs(sitemap, redirects, css, home, &files).join("\n");
    assert!(bad.contains("docs/gone/: no built file"), "{bad}");
    assert!(bad.contains("/a -> /nowhere.html"), "{bad}");
    assert!(bad.contains("url(fonts/missing.woff2)"), "{bad}");
    assert!(!bad.contains("elsewhere"), "{bad}");
    assert_eq!(bad.lines().count(), 3, "{bad}");
}
