//! Guards on the settings routes: absent on a public server and in an export, refused without
//! the page's token or from another site with the file untouched, never naming a path, and a
//! save reports the keys that need a restart.

use std::path::{Path, PathBuf};

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::app::Env;

fn repo(root: &Path) -> PathBuf {
    let p = root.join("r");
    let built = gitterrace_testkit::Fixture::init(&p)
        .write("a.txt", "a\n")
        .commit("one");
    built.path().to_path_buf()
}

/// A server over one repository with `dir` as its config folder.
fn server(root: &Path, dir: &Path, public: bool) -> axum::Router {
    let mut cfg = gitterrace_config::Config::default();
    cfg.server.is_public = public;
    let mut env = Env::new(cfg);
    env.settings_dir = Some(dir.to_path_buf());
    crate::build_server(vec![repo(root)], None, None, &env, None, None)
        .expect("router")
        .0
}

struct Req<'a> {
    method: &'a str,
    uri: &'a str,
    body: String,
    token: Option<&'a str>,
    site: Option<&'a str>,
    origin: Option<String>,
}

impl<'a> Req<'a> {
    fn new(method: &'a str, uri: &'a str, token: Option<&'a str>) -> Self {
        Self {
            method,
            uri,
            body: String::new(),
            token,
            site: Some("same-origin"),
            origin: Some(format!(
                "http://{}",
                gitterrace_testkit::on_port("localhost")
            )),
        }
    }
}

async fn send(r: &axum::Router, q: Req<'_>) -> (u16, String) {
    let mut b = Request::builder()
        .method(q.method)
        .uri(q.uri)
        .header("host", gitterrace_testkit::on_port("localhost"))
        .header("content-type", "application/json");
    if let Some(s) = q.site {
        b = b.header("sec-fetch-site", s);
    }
    if let Some(o) = q.origin {
        b = b.header("origin", o);
    }
    if let Some(t) = q.token {
        b = b.header("x-gitterrace-token", t);
    }
    let res = r
        .clone()
        .oneshot(b.body(Body::from(q.body)).expect("req"))
        .await
        .expect("res");
    let s = res.status().as_u16();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    (s, String::from_utf8_lossy(&bytes).into_owned())
}

/// The token the last server built on this thread hands its pages.
fn token_of(_built: &axum::Router) -> String {
    super::LAST_TOKEN.with(|t| t.borrow().clone())
}

#[tokio::test]
async fn settings_routes_are_absent_under_public() {
    let (root, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    std::fs::write(dir.path().join("config.toml"), "").expect("w");
    let app = server(root.path(), dir.path(), true);
    for (m, uri) in [
        ("GET", "/_settings/file?f=config"),
        ("PUT", "/_settings/file"),
        ("POST", "/_settings/check"),
    ] {
        let (s, _) = send(&app, Req::new(m, uri, Some("x"))).await;
        assert!(
            s == 404 || s == 405,
            "{m} {uri} answered {s} on a public server"
        );
    }
    assert_eq!(
        std::fs::read(dir.path().join("config.toml")).expect("r"),
        b""
    );
}

#[tokio::test]
async fn absent_in_export_and_without_a_config_folder() {
    let root = tempfile::tempdir().expect("t");
    let changes = std::sync::Arc::new(gitterrace_git::ChangeMemo::new());
    let ex = crate::export_router(
        vec![repo(root.path())],
        &changes,
        gitterrace_config::Config::default(),
    )
    .expect("export");
    let (s, _) = send(&ex, Req::new("GET", "/_settings/file?f=config", None)).await;
    assert_eq!(s, 404, "an export serves no settings route");
    let plain = crate::multi_router(vec![root.path().join("r")]).expect("r");
    let (s, _) = send(&plain, Req::new("GET", "/_settings/file?f=config", None)).await;
    assert_eq!(s, 404, "no config folder, no route");
}

#[tokio::test]
async fn cross_site_or_tokenless_put_is_refused_and_file_unchanged() {
    let (root, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "# mine\n").expect("w");
    let app = server(root.path(), dir.path(), false);
    let token = token_of(&app);
    let stamp = gitterrace_config::Stamp::tag(gitterrace_config::Stamp::of(&file).expect("s"));
    let text = "[watch]\ninterval_ms = 500\n";
    let body = serde_json::json!({ "f": "config", "text": text, "stamp": stamp }).to_string();
    let foreign = Some("http://evil.example".to_owned());
    for (why, tok, site, origin) in [
        ("no token", None, Some("same-origin"), None),
        ("wrong token", Some("nope"), Some("same-origin"), None),
        ("cross-site", Some(token.as_str()), Some("cross-site"), None),
        ("no fetch metadata", Some(token.as_str()), None, None),
        (
            "foreign origin",
            Some(token.as_str()),
            Some("same-origin"),
            foreign,
        ),
    ] {
        let mut req = Req::new("PUT", "/_settings/file", tok);
        req.body.clone_from(&body);
        req.site = site;
        if let Some(o) = origin {
            req.origin = Some(o);
        }
        let (s, _) = send(&app, req).await;
        assert_eq!(s, 403, "{why}");
        assert_eq!(
            std::fs::read_to_string(&file).expect("r"),
            "# mine\n",
            "{why}"
        );
    }
    let mut ok = Req::new("PUT", "/_settings/file", Some(&token));
    ok.body = body;
    let (status, reply) = send(&app, ok).await;
    assert_eq!(status, 200, "{reply}");
    assert_eq!(
        std::fs::read_to_string(&file).expect("r"),
        "[watch]\ninterval_ms = 500\n"
    );
}

#[tokio::test]
async fn the_request_cannot_name_a_path() {
    let (root, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    let app = server(root.path(), dir.path(), false);
    let token = token_of(&app);
    for f in ["../config", "/etc/passwd", "config.toml", "Config", ""] {
        let uri = format!("/_settings/file?f={}", gitterrace_testkit::matrix::enc(f));
        let (s, _) = send(&app, Req::new("GET", &uri, Some(&token))).await;
        assert_eq!(s, 400, "{f:?} read");
        let mut req = Req::new("PUT", "/_settings/file", Some(&token));
        req.body = serde_json::json!({ "f": f, "text": "", "stamp": "none" }).to_string();
        let (s, _) = send(&app, req).await;
        assert_eq!(s, 400, "{f:?} write");
    }
    let mut req = Req::new("PUT", "/_settings/file", Some(&token));
    req.body = serde_json::json!({ "f": "config", "path": "/tmp/x", "text": "", "stamp": "none" })
        .to_string();
    let (s, _) = send(&app, req).await;
    assert_eq!(
        s, 200,
        "an unknown field is ignored, the target stays config.toml"
    );
    assert!(dir.path().join("config.toml").exists());
}

#[tokio::test]
async fn put_reports_restart_keys() {
    let (root, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    let app = server(root.path(), dir.path(), false);
    let token = token_of(&app);
    let mut req = Req::new("PUT", "/_settings/file", Some(&token));
    req.body = serde_json::json!({
        "f": "config",
        "changes": [
            { "id": "multi.depth", "value": "3" },
            { "id": "watch.interval_ms", "value": "500" }
        ],
        "stamp": "none"
    })
    .to_string();
    let (status, reply) = send(&app, req).await;
    assert_eq!(status, 200, "{reply}");
    let v: serde_json::Value = serde_json::from_str(&reply).expect("json");
    assert_eq!(v["restart"], serde_json::json!(["multi.depth"]));
    assert!(v["stamp"].as_str().is_some_and(|t| t != "none"));
}

#[tokio::test]
async fn a_stale_or_invalid_save_leaves_the_file() {
    let (root, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "# mine\n").expect("w");
    let app = server(root.path(), dir.path(), false);
    let token = token_of(&app);
    let mut req = Req::new("PUT", "/_settings/file", Some(&token));
    req.body = serde_json::json!({ "f": "config", "text": "", "stamp": "none" }).to_string();
    assert_eq!(
        send(&app, req).await.0,
        409,
        "the file exists, so `none` is stale"
    );
    let stamp = gitterrace_config::Stamp::tag(gitterrace_config::Stamp::of(&file).expect("s"));
    let mut req = Req::new("PUT", "/_settings/file", Some(&token));
    req.body =
        serde_json::json!({ "f": "config", "text": "[watch]\ninterval_ms = 1\n", "stamp": stamp })
            .to_string();
    let (status, reply) = send(&app, req).await;
    assert_eq!(status, 422, "{reply}");
    assert!(reply.contains("\"line\":2"), "{reply}");
    assert_eq!(std::fs::read_to_string(&file).expect("r"), "# mine\n");
}
