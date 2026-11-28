//! Embedded stylesheets, scripts, fonts and the vendored Mermaid bundle.

use axum::{
    Router,
    extract::Path,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};

/// The diagram library: a page's script loads it by name, so no link names it.
pub const MERMAID_JS: &str = "/static/vendor/mermaid-12.3.0.min.js";

/// The scripts and styles `app.js` adds on first use: no page links them, so `export` seeds them.
pub const LAZY_ASSETS: [&str; 16] = [
    "/static/keys.json",
    "/static/actions.js",
    "/static/send.js",
    "/static/send.css",
    "/static/pickers.js",
    "/static/outline.js",
    "/static/diffs.js",
    "/static/diffs-viewed.js",
    "/static/diffs-find.js",
    "/static/diffs-rail.js",
    "/static/card.js",
    "/static/card.css",
    "/static/checkpoints.js",
    "/static/checkpoints.css",
    "/static/basket.js",
    "/static/basket.css",
];

const JS: &str = "text/javascript; charset=utf-8";
const JSON: &str = "application/json; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";

/// A year: a versioned asset URL names one content, so a browser keeps it this long.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// Every embedded script and stylesheet: (name under `/static/`, type, body).
const TABLE: [(&str, &str, &str); 39] = [
    ("loc.js", JS, include_str!("../../static/loc.js")),
    ("tokens.css", CSS, include_str!("../../static/tokens.css")),
    ("app.css", CSS, include_str!("../../static/app.css")),
    ("pagehead.css", CSS, include_str!("../../static/pagehead.css")),
    ("screens.css", CSS, include_str!("../../static/screens.css")),
    ("charts.css", CSS, include_str!("../../static/charts.css")),
    ("insights.css", CSS, include_str!("../../static/insights.css")),
    ("tip.js", JS, include_str!("../../static/tip.js")),
    ("app.js", JS, include_str!("../../static/app.js")),
    ("markdown.css", CSS, include_str!("../../static/markdown.css")),
    ("syntax.css", CSS, include_str!("../../static/syntax.css")),
    ("media.css", CSS, include_str!("../../static/media.css")),
    ("send.css", CSS, include_str!("../../static/send.css")),
    ("buttons.css", CSS, include_str!("../../static/buttons.css")),
    ("docnav.css", CSS, include_str!("../../static/docnav.css")),
    ("diffview.css", CSS, include_str!("../../static/diffview.css")),
    ("graph.css", CSS, include_str!("../../static/graph.css")),
    ("card.js", JS, include_str!("../../static/card.js")),
    ("card.css", CSS, include_str!("../../static/card.css")),
    ("checkpoints.js", JS, include_str!("../../static/checkpoints.js")),
    ("checkpoints.css", CSS, include_str!("../../static/checkpoints.css")),
    ("basket.js", JS, include_str!("../../static/basket.js")),
    ("basket.css", CSS, include_str!("../../static/basket.css")),
    ("font.js", JS, include_str!("../../static/font.js")),
    ("markdown.js", JS, include_str!("../../static/markdown.js")),
    ("outline.js", JS, include_str!("../../static/outline.js")),
    ("diffs.js", JS, include_str!("../../static/diffs.js")),
    ("diffs-viewed.js", JS, include_str!("../../static/diffs-viewed.js")),
    ("diffs-find.js", JS, include_str!("../../static/diffs-find.js")),
    ("diffs-rail.js", JS, include_str!("../../static/diffs-rail.js")),
    ("keys.json", JSON, include_str!("../../static/keys.json")),
    ("nav.js", JS, include_str!("../../static/nav.js")),
    ("pickers.js", JS, include_str!("../../static/pickers.js")),
    ("actions.js", JS, include_str!("../../static/actions.js")),
    ("send.js", JS, include_str!("../../static/send.js")),
    ("live.js", JS, include_str!("../../static/live.js")),
    ("home.js", JS, include_str!("../../static/home.js")),
    ("settings.css", CSS, include_str!("../../static/settings.css")),
    ("settings.js", JS, include_str!("../../static/settings.js")),
];

/// The version every `/static/` URL carries as `?v=`, a hash over all embedded assets.
///
/// A change to any one gives every page new URLs, so a browser never pairs a fresh page with a
/// stale script, stylesheet or font. One value, so a script can reuse its own query for the
/// files it loads.
pub fn asset_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        use std::hash::{Hash as _, Hasher as _};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (name, _, body) in TABLE {
            (name, body).hash(&mut h);
        }
        for (name, body) in FONTS {
            (name, body).hash(&mut h);
        }
        MERMAID.hash(&mut h);
        format!("{:012x}", h.finish() & 0xffff_ffff_ffff)
    });
    &V
}

/// `/static/<name>?v=<version>`, the one way a page names an embedded asset.
pub fn asset_url(name: &str) -> String {
    format!("/static/{name}?v={}", asset_version())
}

/// A stylesheet before minifying: `tokens.css` has its font URLs carry the version, so a new
/// font is fetched.
fn served_source(name: &str, body: &str) -> String {
    if name == "tokens.css" {
        body.replace(".woff2)", &format!(".woff2?v={})", asset_version()))
    } else {
        body.to_owned()
    }
}

/// The assets as served, built once: every stylesheet minified, and `tokens.css` with its font
/// URLs carrying the version too, so a new font is fetched.
fn served(name: &str, kind: &str, body: &'static str) -> &'static str {
    static SERVED: std::sync::LazyLock<std::collections::HashMap<&'static str, String>> =
        std::sync::LazyLock::new(|| {
            TABLE
                .iter()
                .filter(|(_, kind, _)| *kind == CSS)
                .map(|(name, _, body)| {
                    let body = served_source(name, body);
                    (*name, crate::minify::css(&body))
                })
                .collect()
        });
    if kind != CSS {
        return body;
    }
    SERVED.get(name).map_or(body, String::as_str)
}

/// The stylesheets every page loads, joined in cascade order into one request each: `base.css`
/// before the page's own sheet, `late.css` after it. A bundle is its parts back to back, so no
/// rule moves relative to another.
const BUNDLES: [(&str, &[&str]); 3] = [
    (
        "base.css",
        &[
            "tokens.css",
            "buttons.css",
            "app.css",
            "pagehead.css",
            "screens.css",
            "charts.css",
            "syntax.css",
        ],
    ),
    ("late.css", &["media.css", "docnav.css", "diffview.css"]),
    // The same, led by the rendered-markdown rules, for the pages that render markdown.
    ("late-md.css", &["markdown.css", "media.css", "docnav.css", "diffview.css"]),
];

/// The bundle `name` as served, minified, or `None` for a name that is no bundle.
fn bundled(name: &str) -> Option<&'static str> {
    static BUILT: std::sync::LazyLock<Vec<(&'static str, String)>> =
        std::sync::LazyLock::new(|| {
            BUNDLES
                .iter()
                .map(|(name, parts)| {
                    let all: String = parts
                        .iter()
                        .filter_map(|p| TABLE.iter().find(|(n, _, _)| n == p))
                        .map(|(n, _, b)| served_source(n, b))
                        .collect::<Vec<_>>()
                        .join("\n");
                    (*name, crate::minify::css(&all))
                })
                .collect()
        });
    BUILT.iter().find(|(n, _)| *n == name).map(|(_, b)| b.as_str())
}

/// Embedded stylesheets, scripts and fonts; stateless, so the multi-repo index serves them too.
pub(crate) fn assets<S: Clone + Send + Sync + 'static>() -> Router<S> {
    TABLE
        .into_iter()
        .fold(Router::new(), |r, (name, kind, body)| {
            r.route(&format!("/static/{name}"), static_asset(kind, served(name, kind, body)))
        })
        .route("/static/base.css", static_asset(CSS, bundled("base.css").unwrap_or_default()))
        .route("/static/late.css", static_asset(CSS, bundled("late.css").unwrap_or_default()))
        .route("/static/late-md.css", static_asset(CSS, bundled("late-md.css").unwrap_or_default()))
        .route(crate::MERMAID_JS, get(mermaid_js))
        .route("/static/fonts/{name}", get(font))
}

/// Vendored OFL variable fonts, Latin subsets: Fraunces (display), Bricolage Grotesque (body)
/// and `JetBrains` Mono (code).
const FONTS: [(&str, &[u8]); 3] = [
    (
        "fraunces.woff2",
        include_bytes!(
            "../../../../vendor/schwarzwald/kit/fonts/fraunces/fraunces-latin-wght-normal.woff2"
        ),
    ),
    (
        "bricolage-grotesque.woff2",
        include_bytes!(concat!(
            "../../../../vendor/schwarzwald/kit/fonts/",
            "bricolage-grotesque/bricolage-grotesque-latin-wght-normal.woff2"
        )),
    ),
    (
        "jetbrains-mono.woff2",
        include_bytes!(concat!(
            "../../../../vendor/schwarzwald/kit/fonts/",
            "jetbrains-mono/jetbrains-mono-latin-wght-normal.woff2"
        )),
    ),
];

/// The bytes of the embedded font served as `/static/fonts/<name>`, if there is one.
pub fn font_bytes(name: &str) -> Option<&'static [u8]> {
    FONTS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

async fn font(Path(name): Path<String>) -> Response {
    let Some(body) = font_bytes(&name) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("font/woff2")),
            (header::CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE)),
        ],
        body,
    )
        .into_response()
}

static MERMAID: &[u8] = include_bytes!("../../static/vendor/mermaid.min.js");

async fn mermaid_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/javascript; charset=utf-8")),
            (header::CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE)),
        ],
        MERMAID,
    )
        .into_response()
}

/// A strong `ETag` from the embedded bytes: stable for one build, changes with the content.
fn etag(body: &str) -> HeaderValue {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut h);
    HeaderValue::from_str(&format!("\"{:016x}-{:x}\"", h.finish(), body.len()))
        .unwrap_or_else(|_| HeaderValue::from_static("\"0\""))
}

/// Route for an embedded file. At the current `?v=` it is kept a year; any other URL (none, or a
/// version from an older build) revalidates: `no-cache` plus an `ETag`, so a repeat costs a 304.
fn static_asset<S: Clone + Send + Sync + 'static>(
    kind: &'static str,
    body: &'static str,
) -> axum::routing::MethodRouter<S> {
    let tag = etag(body);
    get(move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
        let tag = tag.clone();
        let current = uri.query().and_then(|q| q.strip_prefix("v=")) == Some(asset_version());
        async move { asset(&headers, kind, body, tag, current) }
    })
}

fn asset(
    headers: &axum::http::HeaderMap,
    kind: &'static str,
    body: &'static str,
    tag: HeaderValue,
    current: bool,
) -> Response {
    let fresh = headers.get(header::IF_NONE_MATCH).is_some_and(|v| {
        v.as_bytes().split(|b| *b == b',').any(|t| t.trim_ascii() == tag.as_bytes())
    });
    let meta = [
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static(if current { IMMUTABLE } else { "no-cache" }),
        ),
        (header::ETAG, tag),
    ];
    if fresh {
        return (axum::http::StatusCode::NOT_MODIFIED, meta).into_response();
    }
    ([(header::CONTENT_TYPE, HeaderValue::from_static(kind))], meta, body).into_response()
}
