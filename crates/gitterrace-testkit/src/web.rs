use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// GETs `uri` from a copy of `router` as a request for `localhost`; returns status and body.
pub async fn get(router: &Router, uri: &str) -> (u16, String) {
    let req = Request::get(uri)
        .header("host", "localhost")
        .body(Body::empty())
        .expect("request");
    let res = router.clone().oneshot(req).await.expect("response");
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// GETs page path `uri` (`/log/HEAD`) of the lone repository `router` serves, under the
/// `/r/NAME` prefix `/` forwards to, as a browser arriving at `/` would find it.
pub async fn page(router: &Router, uri: &str) -> (u16, String) {
    get(router, &format!("{}{uri}", base(router).await)).await
}

/// The `/r/NAME` prefix of the lone repository `router` serves, read from where `/` forwards.
pub async fn base(router: &Router) -> String {
    let req = Request::get("/")
        .header("host", "localhost")
        .body(Body::empty())
        .expect("request");
    let res = router.clone().oneshot(req).await.expect("response");
    let to = res
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("/ forwards to the one repository");
    to.trim_end_matches('/').to_owned()
}
