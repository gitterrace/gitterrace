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
/// the shared bar, 404 included.
fn chrome_problems(pages: &[(String, String)]) -> Vec<String> {
    let root = crate::repo_root();
    let ws = crate::facts::load(&root).expect("facts");
    let pkg = &ws.packages["gitterrace"];
    let shared = bar::bar(&pkg.name, &pkg.repository, &ws.demo);
    let log = std::fs::read_to_string(root.join("CHANGELOG.md")).expect("changelog");
    let version_href = changelog::version_href(&log, &pkg.version);
    let site = frame::Site {
        home: "",
        repo: &pkg.repository,
        name: &pkg.name,
        demo: &ws.demo,
        license: &pkg.license,
        version: &pkg.version,
        version_href: &version_href,
    };
    let shared_footer = frame::footer(&site, None);
    let mut bad = Vec::new();
    for (path, html) in pages {
        let Some(foot) = footer_of(html) else {
            bad.push(format!("{path}: not exactly one footer"));
            continue;
        };
        // Every footer opens with the shared links the builder writes for a page with no source.
        let head = shared_footer.trim_end_matches("</footer>\n");
        let head = head.strip_prefix("<footer>").unwrap_or(head);
        if !foot.starts_with(head) {
            bad.push(format!("{path}: the footer is not the shared one"));
        }
        if !html.contains(&shared) {
            bad.push(format!("{path}: the shared bar is missing or changed"));
        }
    }
    bad
}

/// The class this guards: one page whose footer or bar drifted from the single builder.
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
    pages[1].1 = pages[1].1.replacen(
        "Docs</a>\n<a href=\"/changelog/\">",
        "</a>\n<a href=\"/changelog/\">",
        1,
    );
    pages[2].1 = pages[2].1.replacen("<nav ", "<nav class=\"x\" ", 1);
    let bad = chrome_problems(&pages).join("\n");
    assert!(bad.contains("not the shared one"), "{bad}");
    assert!(bad.contains("shared bar is missing"), "{bad}");
}

/// Built mode: pages whose footer does not link both legal pages at their site paths.
fn legal_link_problems(pages: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages {
        let foot = footer_of(html).unwrap_or("");
        for want in ["/legal/imprint/", "/legal/privacy/"] {
            if !attrs(foot, "href").contains(&want) {
                bad.push(format!("{path}: the footer does not link {want}"));
            }
        }
    }
    bad
}

/// The class this guards: a page, on any template, without the imprint and privacy links, and
/// a link to a legal page the build does not write.
#[test]
fn every_built_page_links_the_imprint_and_privacy_pages() {
    let (pages, files) = generated();
    assert!(pages.iter().any(|p| p.0 == "404.html"));
    for f in ["legal/imprint/index.html", "legal/privacy/index.html"] {
        assert!(files.iter().any(|x| x == f), "{f} is not built");
    }
    let bad = legal_link_problems(&pages);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_page_without_a_legal_link_is_reported() {
    let (mut pages, _) = generated();
    pages[3].1 = pages[3]
        .1
        .replacen("href=\"/legal/privacy/\"", "href=\"/\"", 1);
    let bad = legal_link_problems(&pages);
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].contains("/legal/privacy/"), "{bad:?}");
}

/// Links that stay on the site but name a source file, not a page: the site publishes pages, so
/// a file it does not publish must be linked in the repository.
fn site_links_to_files(pages: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages {
        for dest in attrs(html, "href") {
            let file = dest.split('#').next().unwrap_or("");
            let on_site = !file.contains("://") && !file.starts_with("mailto:");
            let source = [".md", ".rs", ".toml", ".sh", ".py", ".json", ".yml"]
                .iter()
                .any(|e| file.ends_with(e));
            if on_site && source {
                bad.push(format!(
                    "{path}: {dest} names a file the site does not publish"
                ));
            }
        }
    }
    bad
}

/// The class this guards: a docs link to a file with no page that points at a site path and so
/// dies, instead of at the repository.
#[test]
fn docs_links_to_unpublished_files_point_at_the_repository() {
    let (pages, _) = generated();
    let bad = site_links_to_files(&pages);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    let links = docs::Links::new("https://github.com/o/r");
    let to = |d: &str| links.rewrite("docs/why.md", d, false);
    assert_eq!(
        to("../src/main.rs"),
        "https://github.com/o/r/blob/main/src/main.rs"
    );
    assert_eq!(to("api.md#x"), "/docs/api/#x");
}

#[test]
fn a_planted_site_link_to_a_source_file_is_reported() {
    let (mut pages, _) = generated();
    pages[0]
        .1
        .push_str("<a href=\"/src/main.rs\">x</a><a href=\"https://h/x.md\">y</a>");
    let bad = site_links_to_files(&pages);
    assert_eq!(bad.len(), 1, "{bad:?}");
}

/// The footer's version link of the landing page must name a heading id that the changelog page
/// has, and the link text must be that version.
fn version_link_problems(landing: &str, changelog: &str, version: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let foot = footer_of(landing).unwrap_or("");
    let want = format!(">v{version}</a>");
    let Some(link) = foot.split("<a ").find(|a| a.contains(&want)) else {
        return vec![format!("the landing footer has no link to v{version}")];
    };
    let tagged = format!(" {link}");
    let href = attrs(&tagged, "href").first().copied().unwrap_or("");
    let Some(anchor) = href.strip_prefix("/changelog/#") else {
        // An unreleased version has no heading yet, and the link then names the page.
        let released = attrs(changelog, "id").contains(&format!("v{version}").as_str());
        return if href == "/changelog/" && !released {
            Vec::new()
        } else {
            vec![format!(
                "the version link {href} is not the anchor of v{version}"
            )]
        };
    };
    if !attrs(changelog, "id").contains(&anchor) {
        bad.push(format!("the changelog has no id {anchor} for v{version}"));
    }
    bad
}

/// The class this guards: a footer version that links to a release the changelog page does not
/// have, or to nothing.
#[test]
fn the_footer_version_links_to_its_changelog_release() {
    let (pages, _) = generated();
    let page = |p: &str| {
        pages
            .iter()
            .find(|x| x.0 == p)
            .map(|x| x.1.clone())
            .expect(p)
    };
    let ws = crate::facts::load(&crate::repo_root()).expect("facts");
    let version = ws.packages["gitterrace"].version.clone();
    let bad = version_link_problems(&page("index.html"), &page("changelog/index.html"), &version);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_footer_version_without_its_changelog_heading_is_reported() {
    let landing = "<footer><a href=\"/changelog/#v9.9.9\">v9.9.9</a></footer>";
    let changelog = "<h2 id=\"v0.1.0\">x</h2>";
    let bad = version_link_problems(landing, changelog, "9.9.9");
    assert!(bad[0].contains("no id v9.9.9"), "{bad:?}");
    let released = "<h2 id=\"v9.9.9\">x</h2>";
    let page = "<footer><a href=\"/changelog/\">v9.9.9</a></footer>";
    let bad = version_link_problems(page, released, "9.9.9");
    assert!(bad[0].contains("not the anchor of v9.9.9"), "{bad:?}");
    assert!(version_link_problems(page, changelog, "9.9.9").is_empty());
    let bad = version_link_problems(landing, changelog, "1.0.0");
    assert!(bad[0].contains("no link to v1.0.0"), "{bad:?}");
}

/// Whether the landing page has the safety section, with the default address the code uses and
/// the link to SECURITY.md in the repository.
fn safety_problems(landing: &str, addr: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let Some((_, rest)) = landing.split_once("<h2>What it does with your code</h2>") else {
        return vec!["the landing page has no \"What it does with your code\" section".to_owned()];
    };
    let section = rest.split("<h2>").next().unwrap_or("");
    if !section.contains(&format!("<code>{addr}</code>")) {
        bad.push(format!("the safety section does not name {addr}"));
    }
    if !section.contains("/blob/main/SECURITY.md") {
        bad.push("the safety section does not link SECURITY.md in the repository".to_owned());
    }
    bad
}

/// The class this guards: the landing page losing the promise section, or stating an address
/// other than the one the code serves on. The prose stays in docs/why.md next to SECURITY.md,
/// whose long list of promises does not reduce to a short generated block, and
/// `why_prose_that_names_a_wrong_address_or_cache_path_fails` ties its facts to the code.
#[test]
fn the_built_landing_page_has_the_safety_section_with_the_real_address() {
    let (pages, _) = generated();
    let landing = &pages
        .iter()
        .find(|p| p.0 == "index.html")
        .expect("landing")
        .1;
    let bad = safety_problems(landing, gitterrace_config::DEFAULT_ADDR);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_landing_page_without_the_section_or_with_another_address_is_reported() {
    let (pages, _) = generated();
    let landing = &pages
        .iter()
        .find(|p| p.0 == "index.html")
        .expect("landing")
        .1;
    let gone = landing.replace("What it does with your code", "Safety");
    assert!(safety_problems(&gone, "127.0.0.1:4242")[0].contains("has no"));
    let bad = safety_problems(landing, "127.0.0.1:9");
    assert!(bad[0].contains("does not name 127.0.0.1:9"), "{bad:?}");
}
