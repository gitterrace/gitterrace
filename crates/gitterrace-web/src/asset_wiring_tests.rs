//! Wiring tests for static assets: every file a template loads is served, and cached by version.

use axum::{body::Body, http::Request};
use gitterrace_config::Config;
use gitterrace_testkit::page as get;
use tower::ServiceExt;

use crate::wiring_tests::{repo, router};

/// Every `/static/...` file a template asks for is served. A script added to a template but
/// not to the asset table 404s, and the browser then refuses it for its empty MIME type.
#[tokio::test]
async fn every_static_file_a_template_loads_is_served() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let r = router(&p, Config::default());
    let templates =
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/templates")).expect("templates");
    let mut wanted = std::collections::BTreeSet::new();
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
        let (code, _) = get(&r, &path).await;
        assert_eq!(code, 200, "{path} is linked from a template but not served");
    }
}

/// An asset at the build's `?v=` is kept a year, any other URL of it revalidates: the class is
/// a page pairing a fresh build with a stale script or style, and a URL without the version
/// that a browser would keep past the next build.
#[tokio::test]
async fn a_versioned_asset_is_immutable_and_an_unversioned_one_revalidates() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let r = router(&p, Config::default());
    let cache = |res: &axum::response::Response| {
        res.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    for (uri, want) in [
        (
            crate::asset_url("app.css"),
            "public, max-age=31536000, immutable",
        ),
        ("/static/app.css".to_owned(), "no-cache"),
        ("/static/app.css?v=old".to_owned(), "no-cache"),
    ] {
        let req = Request::get(&uri)
            .header("host", "localhost")
            .body(Body::empty())
            .expect("req");
        let res = r.clone().oneshot(req).await.expect("send");
        assert_eq!(
            (res.status().as_u16(), cache(&res).as_str()),
            (200, want),
            "{uri}"
        );
    }
    let (_, page) = get(&r, "/").await;
    let v = format!("?v={}", crate::asset_version());
    let (_, css) = get(&r, &crate::asset_url("tokens.css")).await;
    assert!(
        css.contains(&format!(".woff2{v})")),
        "fonts carry the version"
    );
    assert!(
        page.contains(&format!("app.js{v}")),
        "pages link the versioned URL"
    );
}
