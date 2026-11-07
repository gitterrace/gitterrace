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
    let table: [(&str, &str, &'static str); 19] = [
        ("loc.js", JS, include_str!("../../static/loc.js")),
        ("tokens.css", CSS, include_str!("../../static/tokens.css")),
        ("app.css", CSS, include_str!("../../static/app.css")),
        (
            "insights.css",
            CSS,
            include_str!("../../static/insights.css"),
        ),
        ("tip.js", JS, include_str!("../../static/tip.js")),
        ("app.js", JS, include_str!("../../static/app.js")),
        (
            "markdown.css",
            CSS,
            include_str!("../../static/markdown.css"),
        ),
        ("media.css", CSS, include_str!("../../static/media.css")),
        ("send.css", CSS, include_str!("../../static/send.css")),
        ("docnav.css", CSS, include_str!("../../static/docnav.css")),
        ("markdown.js", JS, include_str!("../../static/markdown.js")),
        ("outline.js", JS, include_str!("../../static/outline.js")),
        ("nav.js", JS, include_str!("../../static/nav.js")),
        ("actions.js", JS, include_str!("../../static/actions.js")),
        ("send.js", JS, include_str!("../../static/send.js")),
        ("live.js", JS, include_str!("../../static/live.js")),
        ("home.js", JS, include_str!("../../static/home.js")),
        (
            "settings.css",
            CSS,
            include_str!("../../static/settings.css"),
        ),
        ("settings.js", JS, include_str!("../../static/settings.js")),
    ];
    table
        .into_iter()
        .fold(Router::new(), |r, (name, kind, body)| {
            r.route(&format!("/static/{name}"), static_asset(kind, body))
        })
        .route(crate::MERMAID_JS, get(mermaid_js))
        .route("/static/fonts/{name}", get(font))
}

/// Vendored OFL variable fonts, Latin subsets: Fraunces (display), Bricolage Grotesque (body)
/// and `JetBrains` Mono (code).
const FONTS: [(&str, &[u8]); 3] = [
    (
        "fraunces.woff2",
        include_bytes!(
            "../../../../shared/schwarzwald/fonts/fraunces/fraunces-latin-wght-normal.woff2"
        ),
    ),
    (
        "bricolage-grotesque.woff2",
        include_bytes!(concat!(
            "../../../../shared/schwarzwald/fonts/",
            "bricolage-grotesque/bricolage-grotesque-latin-wght-normal.woff2"
        )),
    ),
    (
        "jetbrains-mono.woff2",
        include_bytes!(concat!(
            "../../../../shared/schwarzwald/fonts/",
            "jetbrains-mono/jetbrains-mono-latin-wght-normal.woff2"
        )),
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

/// Revalidating route for an embedded file: `no-cache` plus an `ETag`, so a repeat visit costs a
/// 304.
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

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// `/static/<name>` for every script and stylesheet a template names, so a page never
    /// loads a file the server does not serve (a script that silently never runs).
    fn named_in(text: &str) -> Vec<String> {
        text.split("/static/")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .filter(|n| {
                let ext = std::path::Path::new(n).extension().and_then(|e| e.to_str());
                matches!(ext, Some("js" | "css")) && !n.contains('{') && !n.contains('/')
            })
            .map(str::to_owned)
            .collect()
    }

    async fn missing(names: &[String]) -> Vec<String> {
        let r: axum::Router = super::assets();
        let mut out = Vec::new();
        for n in names {
            let req = Request::get(format!("/static/{n}"))
                .body(Body::empty())
                .expect("req");
            let res = r.clone().oneshot(req).await.expect("res");
            if res.status() != 200 {
                out.push(n.clone());
            }
        }
        out
    }

    #[tokio::test]
    async fn every_script_and_stylesheet_a_template_names_is_served() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let mut names = Vec::new();
        for e in std::fs::read_dir(dir).expect("templates") {
            let text = std::fs::read_to_string(e.expect("entry").path()).expect("read");
            names.extend(named_in(&text));
        }
        names.sort();
        names.dedup();
        assert!(names.len() >= 5, "the scan found the assets: {names:?}");
        assert_eq!(missing(&names).await, Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_template_naming_an_unserved_script_is_reported() {
        let names = named_in(r#"<script src="/static/planted.js" defer></script>"#);
        assert_eq!(missing(&names).await, ["planted.js"]);
    }
}
