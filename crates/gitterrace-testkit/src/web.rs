use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// GETs `uri` from a copy of `router` as a request for `localhost`; returns status and body.
pub async fn get(router: &Router, uri: &str) -> (u16, String) {
    let req = Request::get(uri).header("host", "localhost").body(Body::empty()).expect("request");
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
    let req = Request::get("/").header("host", "localhost").body(Body::empty()).expect("request");
    let res = router.clone().oneshot(req).await.expect("response");
    let to = res
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("/ forwards to the one repository");
    to.trim_end_matches('/').to_owned()
}

/// Header destinations a page gives more than once: every `href` in the header's tool area plus
/// the `gt-settings` menu meta, which the actions menu turns into an entry.
pub fn repeated_header_destinations(body: &str) -> Vec<String> {
    let head = body
        .split("<header")
        .nth(1)
        .and_then(|h| h.split("</header>").next())
        .and_then(|h| h.split("class=\"tools\"").nth(1))
        .unwrap_or_default();
    let meta =
        body.split("name=\"gt-settings\" content=\"").nth(1).and_then(|m| m.split('"').next());
    let dests = head.split(" href=\"").skip(1).filter_map(|p| p.split('"').next()).chain(meta);
    let mut seen = std::collections::BTreeSet::new();
    let twice: std::collections::BTreeSet<String> =
        dests.filter(|d| !d.is_empty() && !seen.insert(*d)).map(str::to_owned).collect();
    twice.into_iter().collect()
}

/// Ids an HTML body gives more than once. A repeated id sends a `#fragment`, a label or a
/// script to whichever element comes first.
pub fn duplicate_ids(body: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut twice = std::collections::BTreeSet::new();
    for part in body.split(" id=\"").skip(1) {
        let id = part.split('"').next().unwrap_or_default();
        if !seen.insert(id) {
            twice.insert(id.to_owned());
        }
    }
    twice.into_iter().collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_settings_link_beside_the_settings_meta_is_a_repeat() {
        let gear = concat!(
            r#"<meta name="gt-settings" content="/_settings"><header><div class="tools">"#,
            r#"<a id="settingsbtn" href="/_settings"></a></div></header>"#
        );
        assert_eq!(super::repeated_header_destinations(gear), ["/_settings"]);
    }

    #[test]
    fn an_id_given_twice_is_reported_once() {
        let body = r#"<a id="x"></a><b id="y"></b><i id="x"></i>"#;
        assert_eq!(super::duplicate_ids(body), ["x"]);
    }
}
