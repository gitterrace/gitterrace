//! Checks over the whole built site, as `site/` will hold it: pages, `_headers`, `_redirects`,
//! the 404 page, `sitemap.xml` and the stylesheet. Each check is a function over data, run on the
//! real output by one test and on a planted fault by another, so it is seen to go red.

use crate::site::tests::{attrs, generated, unresolved};
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

/// The sha256 CSP source of an inline script body.
fn csp_hash(body: &str) -> String {
    use base64::Engine as _;
    use sha2::Digest as _;
    let sum = sha2::Sha256::digest(body.as_bytes());
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(sum)
    )
}

/// Everything the built pages do that the `_headers` CSP would refuse or that weakens it: an
/// inline script without its hash, `unsafe-inline`, an external origin in the policy, a `<style>`
/// or `style=` (style-src is 'self'), a `<script src>`, or a `<link>` off this site.
fn csp_problems(headers: &str, pages: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    let policy = headers
        .lines()
        .find_map(|l| l.trim().strip_prefix("Content-Security-Policy:"))
        .unwrap_or("");
    if policy.is_empty() {
        bad.push("_headers: no Content-Security-Policy".to_owned());
    }
    for tok in policy.split([' ', ';']) {
        if tok.contains("unsafe-") {
            bad.push(format!("_headers: CSP has {tok}"));
        }
        if tok.contains("://") || tok == "*" || tok == "data:" || tok == "blob:" {
            bad.push(format!("_headers: CSP allows the origin {tok}"));
        }
    }
    for directive in ["style-src", "font-src", "img-src"] {
        let ok = policy
            .split(';')
            .any(|d| d.trim().split(' ').eq([directive, "'self'"]));
        if !ok {
            bad.push(format!("_headers: CSP {directive} is not exactly 'self'"));
        }
    }
    for (path, html) in pages {
        for chunk in html.split("<script").skip(1) {
            let (tag, rest) = chunk.split_once('>').unwrap_or((chunk, ""));
            if tag.contains("src=") {
                bad.push(format!("{path}: a script is loaded, not inline"));
                continue;
            }
            let body = rest.split("</script>").next().unwrap_or("");
            if !policy.contains(&csp_hash(body)) {
                bad.push(format!(
                    "{path}: inline script {} is not in the CSP",
                    csp_hash(body)
                ));
            }
        }
        if html.contains("<style") || html.contains(" style=\"") {
            bad.push(format!("{path}: inline style, refused by style-src 'self'"));
        }
        for tag in html.split("<link").skip(1) {
            let tag = tag.split('>').next().unwrap_or("");
            // A canonical link names the page, the browser fetches nothing from it.
            if !tag.contains("rel=\"canonical\"")
                && attrs(&format!("<link{tag}"), "href")
                    .iter()
                    .any(|h| h.contains("://") || h.starts_with("//"))
            {
                bad.push(format!("{path}: a link to another origin: {tag}"));
            }
        }
    }
    bad
}

/// The class this guards: a page whose scripts or styles the policy would block, or a policy
/// widened to let them through.
#[test]
fn every_inline_script_of_every_built_page_is_hashed_in_the_csp() {
    let (pages, _) = generated();
    let bad = csp_problems(&text_of("_headers"), &pages);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_planted_inline_script_or_widened_policy_is_reported() {
    let (mut pages, _) = generated();
    let headers = text_of("_headers");
    pages[0].1.push_str(
        "<script>alert(1)</script><style>a{}</style><script src=\"/x.js\"></script>\
         <link rel=\"stylesheet\" href=\"https://cdn.example/a.css\">",
    );
    let bad = csp_problems(&headers, &pages).join("\n");
    assert!(
        bad.contains("inline script") && bad.contains("not in the CSP"),
        "{bad}"
    );
    assert!(bad.contains("inline style"), "{bad}");
    assert!(bad.contains("is loaded, not inline"), "{bad}");
    assert!(bad.contains("another origin"), "{bad}");
    let wide = headers
        .replace(
            "script-src ",
            "script-src 'unsafe-inline' https://cdn.example ",
        )
        .replace("style-src 'self'", "style-src 'self' https://f.example");
    let bad = csp_problems(&wide, &[]).join("\n");
    assert!(bad.contains("unsafe-inline"), "{bad}");
    assert!(bad.contains("https://cdn.example"), "{bad}");
    assert!(bad.contains("style-src is not exactly 'self'"), "{bad}");
}

/// Every external font or stylesheet reference of the pages and CSS, and every `@font-face` src
/// that is not a built file. Fonts are self-hosted, so nothing may name another origin.
fn font_problems(css: &str, pages: &[(String, String)], files: &[String]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages {
        for tag in html.split("<link").skip(1) {
            let tag = tag.split('>').next().unwrap_or("");
            let external = attrs(&format!("<link{tag}"), "href")
                .iter()
                .any(|h| h.contains("://") || h.starts_with("//"));
            if external && !tag.contains("rel=\"canonical\"") {
                bad.push(format!("{path}: external link:{tag}"));
            }
        }
    }
    for needle in ["@import", "fonts.googleapis", "fonts.gstatic", "://"] {
        if css.contains(needle) {
            bad.push(format!("assets/site.css names {needle}"));
        }
    }
    let faces = css.matches("@font-face").count();
    if faces == 0 {
        bad.push("assets/site.css has no @font-face".to_owned());
    }
    for face in css.split("@font-face").skip(1) {
        let block = face.split('}').next().unwrap_or("");
        let srcs: Vec<&str> = block
            .split("url(")
            .skip(1)
            .filter_map(|u| u.split(')').next())
            .collect();
        if srcs.is_empty() {
            bad.push("assets/site.css: an @font-face without a url()".to_owned());
        }
        for src in srcs {
            let src = src.trim_matches(['"', '\'']);
            if !files.contains(&format!("assets/{src}")) {
                bad.push(format!(
                    "assets/site.css: @font-face src {src} is not built"
                ));
            }
        }
    }
    bad
}

/// The class this guards: a font or stylesheet fetched from a third party, or a face whose file
/// the build does not write.
#[test]
fn fonts_are_self_hosted_and_every_font_face_src_is_built() {
    let (pages, files) = generated();
    let bad = font_problems(&text_of("assets/site.css"), &pages, &files);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_planted_external_font_or_missing_face_file_is_reported() {
    let (mut pages, files) = generated();
    pages[0]
        .1
        .push_str("<link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?family=X\">");
    let css = "@import url(https://x.example/a.css);\
               @font-face{font-family:\"A\";src:url(fonts/nope.woff2)}";
    let bad = font_problems(css, &pages, &files).join("\n");
    assert!(bad.contains("external link"), "{bad}");
    assert!(
        bad.contains("names @import") && bad.contains("names ://"),
        "{bad}"
    );
    assert!(bad.contains("src fonts/nope.woff2 is not built"), "{bad}");
    assert!(
        !font_problems("", &[], &files).is_empty(),
        "no @font-face is red"
    );
}

/// The `<footer>` of a page, if it has exactly one.
fn footer_of(html: &str) -> Option<&str> {
    let (_, rest) = html.split_once("<footer>")?;
    let (inner, tail) = rest.split_once("</footer>")?;
    (!tail.contains("<footer>")).then_some(inner)
}

/// Pages whose footer does not open with the shared links as `footer` builds them, or that lack
/// the shared nav (every page but the 404, which is a dead end with a way back).
fn chrome_problems(pages: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages {
        let Some(foot) = footer_of(html) else {
            bad.push(format!("{path}: not exactly one footer"));
            continue;
        };
        let repo = attrs(foot, "href").first().copied().unwrap_or("");
        let shared = footer(repo, "");
        let shared = shared
            .strip_prefix("<footer>")
            .and_then(|s| s.strip_suffix("</footer>\n"))
            .unwrap_or("");
        if !foot.starts_with(shared) {
            bad.push(format!("{path}: the footer is not the shared one"));
        }
        if path != "404.html" && !html.contains(nav()) {
            bad.push(format!("{path}: the shared nav is missing or changed"));
        }
    }
    bad
}

/// The class this guards: one page whose footer or nav drifted from the single builder.
#[test]
fn every_built_page_has_the_shared_footer_and_nav() {
    let (pages, _) = generated();
    assert!(pages.iter().any(|p| p.0 == "404.html"));
    let bad = chrome_problems(&pages);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_page_with_an_altered_footer_or_nav_is_reported() {
    let (mut pages, _) = generated();
    assert!(chrome_problems(&pages).is_empty());
    pages[1].1 = pages[1]
        .1
        .replacen("\n<a href=\"/docs/\">Docs</a>\n", "\n", 1);
    pages[2].1 = pages[2].1.replacen("<nav ", "<nav class=\"x\" ", 1);
    let bad = chrome_problems(&pages).join("\n");
    assert!(bad.contains("not the shared one"), "{bad}");
    assert!(bad.contains("shared nav is missing"), "{bad}");
}
