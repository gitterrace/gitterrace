//! Embedded stylesheets, scripts, fonts and the vendored Mermaid bundle.

use axum::{
    Router,
    extract::Path,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};

/// Embedded stylesheets, scripts and fonts; stateless, so the multi-repo index serves them too.
pub(crate) fn assets<S: Clone + Send + Sync + 'static>() -> Router<S> {
    const JS: &str = "text/javascript; charset=utf-8";
    const CSS: &str = "text/css; charset=utf-8";
    let table: [(&str, &str, &'static str); 11] = [
        ("loc.js", JS, include_str!("../../static/loc.js")),
        ("tokens.css", CSS, include_str!("../../static/tokens.css")),
        ("app.css", CSS, include_str!("../../static/app.css")),
        ("tip.js", JS, include_str!("../../static/tip.js")),
        ("app.js", JS, include_str!("../../static/app.js")),
        (
            "markdown.css",
            CSS,
            include_str!("../../static/markdown.css"),
        ),
        ("markdown.js", JS, include_str!("../../static/markdown.js")),
        ("nav.js", JS, include_str!("../../static/nav.js")),
        ("actions.js", JS, include_str!("../../static/actions.js")),
        ("send.js", JS, include_str!("../../static/send.js")),
        ("live.js", JS, include_str!("../../static/live.js")),
    ];
    table
        .into_iter()
        .fold(Router::new(), |r, (name, kind, body)| {
            r.route(&format!("/static/{name}"), static_asset(kind, body))
        })
        .route("/static/vendor/mermaid-11.17.2.min.js", get(mermaid_js))
        .route("/static/fonts/{name}", get(font))
}

/// Vendored OFL variable fonts, Latin subsets: Fraunces (display), Bricolage Grotesque (body)
/// and `JetBrains` Mono (code).
const FONTS: [(&str, &[u8]); 3] = [
    (
        "fraunces.woff2",
        include_bytes!(
            "../../../../theme/schwarzwald/fonts/fraunces/fraunces-latin-wght-normal.woff2"
        ),
    ),
    (
        "bricolage-grotesque.woff2",
        include_bytes!(
            "../../../../theme/schwarzwald/fonts/bricolage-grotesque/bricolage-grotesque-latin-wght-normal.woff2"
        ),
    ),
    (
        "jetbrains-mono.woff2",
        include_bytes!(
            "../../../../theme/schwarzwald/fonts/jetbrains-mono/jetbrains-mono-latin-wght-normal.woff2"
        ),
    ),
];

async fn font(Path(name): Path<String>) -> Response {
    let Some((_, body)) = FONTS.iter().find(|(n, _)| *n == name) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("font/woff2")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        *body,
    )
        .into_response()
}

static MERMAID: &[u8] = include_bytes!("../../static/vendor/mermaid.min.js");

async fn mermaid_js() -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/javascript; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
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

/// Revalidating route for an embedded file: `no-cache` plus an `ETag`, so a repeat visit costs a 304.
fn static_asset<S: Clone + Send + Sync + 'static>(
    kind: &'static str,
    body: &'static str,
) -> axum::routing::MethodRouter<S> {
    let tag = etag(body);
    get(move |headers: axum::http::HeaderMap| {
        let tag = tag.clone();
        async move { asset(&headers, kind, body, tag) }
    })
}

fn asset(
    headers: &axum::http::HeaderMap,
    kind: &'static str,
    body: &'static str,
    tag: HeaderValue,
) -> Response {
    let fresh = headers.get(header::IF_NONE_MATCH).is_some_and(|v| {
        v.as_bytes()
            .split(|b| *b == b',')
            .any(|t| t.trim_ascii() == tag.as_bytes())
    });
    let meta = [
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        (header::ETAG, tag),
    ];
    if fresh {
        return (axum::http::StatusCode::NOT_MODIFIED, meta).into_response();
    }
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static(kind))],
        meta,
        body,
    )
        .into_response()
}
