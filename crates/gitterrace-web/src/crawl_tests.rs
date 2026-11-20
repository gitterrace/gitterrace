//! One crawl over a two-repository server checks what every page owes, so a new page is checked
//! the day it is linked: no 5xx, no link leaving its `/r/NAME` (`fmt::href`), the favicon in
//! every shell (error pages and the `/go` miss included), assets at the server root and served,
//! and `tip.js` wherever a hover tip is.

use std::collections::{BTreeSet, HashSet};

use axum::{Router, body::Body, http::Request};
use tower::ServiceExt;

use crate::tests::{fetch, fixture, links};

/// What one page owes, whatever it is. `prefix` is the repository the page belongs to.
fn page_faults(u: &str, body: &str, prefix: Option<&str>) -> Vec<String> {
    let mut bad = Vec::new();
    if !body.contains(crate::fmt::FAVICON.trim()) {
        bad.push(format!("{u}: no favicon"));
    }
    // Stylesheets and scripts are the server's, one URL each across every repository.
    for (at, _) in body.match_indices("/static/") {
        let before = &body[..at];
        if !(before.ends_with('"') || before.ends_with('\'') || before.ends_with('=')) {
            // Cut points searched for: the asset can sit at the page's end, past a multibyte char.
            let from =
                (0..=at.saturating_sub(30)).rev().find(|&i| body.is_char_boundary(i)).unwrap_or(0);
            let to =
                (at + 20..=body.len()).find(|&i| body.is_char_boundary(i)).unwrap_or(body.len());
            let near = &body[from..to.min(body.len())];
            bad.push(format!("{u}: an asset under a prefix: {near}"));
        }
    }
    if body.contains("data-tip=") && !body.contains("/static/tip.js") {
        bad.push(format!("{u}: a hover tip without tip.js"));
    }
    // The brand goes to the repository index and assets are the server's: both at the root on
    // purpose. The Related line's links to a served repository's log are taken out first.
    if let Some(prefix) = prefix {
        for l in links(body) {
            if l != "/" && !l.starts_with("/static/") && !l.starts_with(prefix) {
                bad.push(format!("{u} -> {l} escapes {prefix}"));
            }
        }
    }
    bad
}

/// Takes the Related line's anchors out of `html` when each points at `/r/<name>/log` of a name
/// in `served`, and returns those hrefs and the rest of the page. Any other anchor in the line
/// stays in the page, so the crawl still flags it.
fn related_logs(html: &str, served: &[&str]) -> (Vec<String>, String) {
    const OPEN: &str = "<span class=\"related\">";
    let Some(i) = html.find(OPEN) else {
        return (Vec::new(), html.to_owned());
    };
    let end = html[i..].find("</span>").map_or(html.len(), |e| i + e + "</span>".len());
    let mut span = html[i..end].to_owned();
    let mut hrefs = Vec::new();
    for name in served {
        let href = format!("/r/{name}/log");
        let anchor = format!("<a href=\"{href}\">{name}</a>");
        if span.contains(&anchor) {
            span = span.replace(&anchor, "");
            hrefs.push(href);
        }
    }
    (hrefs, format!("{}{span}{}", &html[..i], &html[end..]))
}

#[test]
fn a_page_that_breaks_a_rule_is_caught() {
    let icon = crate::fmt::FAVICON.trim();
    assert_eq!(page_faults("/x", "<html>", None).len(), 1, "no favicon");
    let prefixed = format!("{icon}<link href=\"/r/a/static/app.css\">");
    assert_eq!(page_faults("/x", &prefixed, Some("/r/a/")).len(), 1);
    let tip = format!("{icon}<b data-tip=\"x\">");
    assert_eq!(page_faults("/x", &tip, None).len(), 1, "tip without tip.js");
    let escape = format!("{icon}<a href=\"/r/b/log\">");
    assert_eq!(page_faults("/x", &escape, Some("/r/a/")).len(), 1);
    let fine = format!("{icon}<a href=\"/\"></a><a href=\"/r/a/log\"></a>");
    assert_eq!(page_faults("/x", &fine, Some("/r/a/")).len(), 0);
    let related = "<span class=\"related\"><a href=\"/r/b/log\">b</a><a href=\"/x\">x</a></span>";
    let (hrefs, rest) = related_logs(related, &["b"]);
    assert_eq!(hrefs, ["/r/b/log"]);
    assert!(rest.contains("/x") && !rest.contains("/r/b/log"));
}

/// Crawls every page of both repositories, then the shells no link reaches.
#[tokio::test]
async fn every_crawled_page_keeps_the_page_rules() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = fixture("alpha", dir.path());
    // A clone, so the two share a root and alpha's log shows the Related line. Two fixtures made
    // apart share one only when their commits land in the same second.
    let b = dir.path().join("beta");
    let (from, to) = (a.to_str().expect("utf8"), b.to_str().expect("utf8"));
    gitterrace_testkit::git(dir.path(), &["clone", "-q", from, to]);
    let router = crate::multi_router(vec![a.clone(), b]).expect("router");
    gitterrace_testkit::wait::until("the Related line on alpha's log", || async {
        fetch(&router, "/r/alpha/log/HEAD", "localhost").await.1.contains("class=\"related\"")
    })
    .await;
    let (s, idx) = fetch(&router, "/", "localhost").await;
    assert_eq!(s, 200);
    assert!(idx.contains("/r/alpha/") && idx.contains("/r/beta/"));
    let mut bad = Vec::new();
    let mut assets = BTreeSet::new();
    let mut tipped = 0;
    for name in ["alpha", "beta"] {
        let prefix = format!("/r/{name}/");
        let mut todo = vec![prefix.clone()];
        let mut seen = HashSet::new();
        let mut seen_related = BTreeSet::new();
        while let Some(u) = todo.pop() {
            if !seen.insert(u.clone()) || seen.len() > 400 {
                continue;
            }
            let (st, body) = fetch(&router, &u, "localhost").await;
            assert!(st < 500, "{u}: {st} {body}");
            if !body.contains("<html") {
                continue;
            }
            let (related, rest) = related_logs(&body, &["alpha", "beta"]);
            if u.starts_with("/r/alpha/log") {
                seen_related.extend(related);
            }
            tipped += usize::from(rest.contains("data-tip="));
            bad.extend(page_faults(&u, &rest, Some(&prefix)));
            for l in links(&rest) {
                let l = l.split('#').next().unwrap_or_default().to_owned();
                if l.starts_with("/static/") {
                    assets.insert(l);
                } else if l.starts_with(&prefix) && !l.contains("/raw/") {
                    todo.push(l);
                }
            }
        }
        assert!(seen.len() > 10, "crawl too shallow: {}", seen.len());
        // The exemption stays live: alpha's log shows the Related line, to beta's log only.
        if name == "alpha" {
            let want: BTreeSet<String> = ["/r/beta/log".to_owned()].into();
            assert_eq!(seen_related, want, "the Related line on alpha's log");
        }
    }
    assert!(tipped > 0, "no crawled page has a hover tip");
    assert!(assets.len() >= 5, "only {assets:?}");
    for asset in &assets {
        let (s, _) = fetch(&router, asset, "localhost").await;
        if s != 200 {
            bad.push(format!("{asset}: linked but answers {s}"));
        }
    }
    // Shells no link reaches: error pages, the `/go` miss, and a lone repository's server.
    let one = crate::app_router(&a).expect("router");
    for (r, u) in [
        (&one, "/r/alpha/"),
        (&one, "/r/alpha/blob/HEAD/no-such-file"),
        (&one, "/r/alpha/no-such-page"),
        (&one, "/no-such-page"),
        (&router, "/"),
        (&router, "/r/alpha/no-such-page"),
        (&router, "/r/no-such-repo/"),
        (&router, "/no-such-page"),
        (&router, "/go/nope/abc"),
    ] {
        let (_, body) = fetch(r, u, "localhost").await;
        bad.extend(page_faults(u, &body, None));
    }
    let api = [
        ("/r/beta/api/files/HEAD", "src/a.rs"),
        ("/r/alpha/api/v1/repo", ""),
        ("/r/alpha/raw/HEAD/src/a.rs", ""),
    ];
    for (u, has) in api {
        let (s, body) = fetch(&router, u, "localhost").await;
        assert!(s == 200 && body.contains(has), "{u}: {s}");
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Every `/static/...` file a template asks for is served, including from a template no crawl
/// reaches. A script added to a template but not to the asset table 404s, and the browser then
/// refuses it for its empty MIME type.
#[tokio::test]
async fn every_static_file_a_template_loads_is_served() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = crate::app_router(fixture("r", dir.path())).expect("router");
    let templates =
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/templates")).expect("templates");
    let mut wanted = BTreeSet::new();
    for entry in templates {
        let text = std::fs::read_to_string(entry.expect("entry").path()).expect("read");
        assert!(
            !text.contains("\"/static/"),
            "a template names /static/ directly: use crate::asset_url"
        );
        for (at, _) in text.match_indices("asset_url(\"") {
            let rest = &text[at + "asset_url(\"".len()..];
            let end = rest.find('"').unwrap_or(rest.len());
            wanted.insert(crate::asset_url(&rest[..end]));
        }
    }
    assert!(wanted.len() >= 5, "found only {wanted:?}");
    for path in wanted {
        let (code, _) = fetch(&r, &path, "localhost").await;
        assert_eq!(code, 200, "{path} is linked from a template but not served");
    }
}

/// An asset at the build's `?v=` is kept a year, any other URL of it revalidates: the class is
/// a page pairing a fresh build with a stale script or style, and a URL without the version
/// that a browser would keep past the next build.
#[tokio::test]
async fn a_versioned_asset_is_immutable_and_an_unversioned_one_revalidates() {
    let dir = tempfile::tempdir().expect("tmp");
    let r: Router = crate::app_router(fixture("r", dir.path())).expect("router");
    for (uri, want) in [
        (crate::asset_url("app.css"), "public, max-age=31536000, immutable"),
        ("/static/app.css".to_owned(), "no-cache"),
        ("/static/app.css?v=old".to_owned(), "no-cache"),
    ] {
        let req = Request::get(&uri).header("host", "localhost").body(Body::empty()).expect("req");
        let res = r.clone().oneshot(req).await.expect("send");
        let cache =
            res.headers().get("cache-control").and_then(|v| v.to_str().ok()).unwrap_or_default();
        assert_eq!((res.status().as_u16(), cache), (200, want), "{uri}");
    }
    let (_, page) = gitterrace_testkit::page(&r, "/").await;
    let v = format!("?v={}", crate::asset_version());
    let (_, css) = fetch(&r, &crate::asset_url("tokens.css"), "localhost").await;
    assert!(css.contains(&format!(".woff2{v})")), "fonts carry the version");
    assert!(page.contains(&format!("app.js{v}")), "pages link the versioned URL");
}
