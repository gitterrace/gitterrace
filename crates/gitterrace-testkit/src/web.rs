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
