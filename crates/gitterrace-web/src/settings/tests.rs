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

/// GETs `uri` the way a browser on this server would.
async fn page(r: &axum::Router, uri: &str) -> (u16, String) {
    let mut req = Req::new("GET", uri, None);
    req.site = None;
    req.origin = None;
    send(r, req).await
}

#[tokio::test]
async fn form_and_plain_saves_produce_the_same_file() {
    const START: &str =
        "# my settings\n[watch]\n# fast\ninterval_ms = 500\n\n[display]\ntheme = \"dark\"\n";
    let mut files = Vec::new();
    for how in ["form", "plain"] {
        let (root, dir) = (
            tempfile::tempdir().expect("t"),
            tempfile::tempdir().expect("t"),
        );
        let file = dir.path().join("config.toml");
        std::fs::write(&file, START).expect("w");
        let app = server(root.path(), dir.path(), false);
        let token = token_of(&app);
        let stamp = gitterrace_config::Stamp::tag(gitterrace_config::Stamp::of(&file).expect("s"));
        let changes = serde_json::json!([
            { "id": "watch.interval_ms", "value": "750" },
            { "id": "insights.exclude", "value": "[\"vendor/**\"]" }
        ]);
        // Plain sends the text the check reply rendered for the same form edit: what the page
        // puts in the editor when the user switches from Form to Plain.
        let body = if how == "form" {
            serde_json::json!({ "f": "config", "changes": changes, "stamp": stamp })
        } else {
            let mut req = Req::new("POST", "/_settings/check", Some(&token));
            req.body = serde_json::json!({ "f": "config", "changes": changes }).to_string();
            let (status, reply) = send(&app, req).await;
            assert_eq!(status, 200, "{reply}");
            let v: serde_json::Value = serde_json::from_str(&reply).expect("json");
            let text = v["text"].as_str().expect("rendered text").to_owned();
            serde_json::json!({ "f": "config", "text": text, "stamp": stamp })
        };
        let mut req = Req::new("PUT", "/_settings/file", Some(&token));
        req.body = body.to_string();
        let (status, reply) = send(&app, req).await;
        assert_eq!(status, 200, "{how}: {reply}");
        files.push(std::fs::read(&file).expect("r"));
    }
    assert_eq!(files[0], files[1], "form and plain wrote different files");
    let text = String::from_utf8_lossy(&files[0]);
    assert!(
        text.starts_with("# my settings\n[watch]\n# fast\ninterval_ms = 750\n"),
        "{text}"
    );
    assert!(text.contains("theme = \"dark\""), "{text}");
}

#[tokio::test]
async fn check_reply_names_the_line() {
    let (root, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    let app = server(root.path(), dir.path(), false);
    let token = token_of(&app);
    for (text, line, key) in [
        (
            "[watch]\nenabled = true\ninterval_ms = 3\n",
            3,
            Some("watch.interval_ms"),
        ),
        ("[watch]\nenabled = true\n[display\n", 3, None),
        ("\n\n[server]\naddr = 4\n", 4, Some("server.addr")),
    ] {
        let mut req = Req::new("POST", "/_settings/check", Some(&token));
        req.body = serde_json::json!({ "f": "config", "text": text }).to_string();
        let (status, reply) = send(&app, req).await;
        assert_eq!(status, 200, "{reply}");
        let v: serde_json::Value = serde_json::from_str(&reply).expect("json");
        assert_eq!(v["ok"], false, "{text:?}");
        assert_eq!(v["errors"][0]["line"], line, "{text:?}: {reply}");
        assert_eq!(v["errors"][0]["key"].as_str(), key, "{text:?}");
    }
    assert!(
        !dir.path().join("config.toml").exists(),
        "a check writes nothing"
    );
}

#[tokio::test]
async fn the_page_renders_the_schema_and_carries_its_token() {
    let (root, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    std::fs::write(
        dir.path().join("config.toml"),
        "[watch]\ninterval_ms = 500\n",
    )
    .expect("w");
    let app = server(root.path(), dir.path(), false);
    let (s, html) = page(&app, "/_settings").await;
    assert_eq!(s, 200, "{html}");
    let token = token_of(&app);
    assert!(html.contains(&format!("name=\"gitterrace-settings\" content=\"{token}\"")));
    for k in gitterrace_config::keys() {
        assert!(
            html.contains(&format!("data-id=\"{}\"", k.id)),
            "{} has no row",
            k.id
        );
    }
    assert!(html.contains("value=\"500\""), "the file's value is shown");
    assert!(!html.contains("style=\""), "no inline style");
    let (s, overview) = page(&app, "/r/r/").await;
    assert_eq!(s, 200);
    assert!(overview.contains("id=\"settingsbtn\""), "the gear shows");
    assert!(
        overview.contains("name=\"gt-settings\""),
        "the menu entry is offered"
    );
    assert!(
        !overview.contains(&token),
        "only the settings page carries its token"
    );
}

#[tokio::test]
async fn without_the_routes_no_page_offers_settings() {
    let root = tempfile::tempdir().expect("t");
    let app = crate::multi_router(vec![repo(root.path())]).expect("r");
    let (s, overview) = page(&app, "/r/r/").await;
    assert_eq!(s, 200);
    assert!(!overview.contains("settingsbtn") && !overview.contains("gt-settings"));
    let (root2, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    let public = server(root2.path(), dir.path(), true);
    let (s, overview) = page(&public, "/r/r/").await;
    assert_eq!(s, 200);
    assert!(
        !overview.contains("settingsbtn"),
        "a public server offers no settings"
    );
}

#[tokio::test]
async fn restart_keys_name_only_what_the_edit_changes() {
    let (root, dir) = (
        tempfile::tempdir().expect("t"),
        tempfile::tempdir().expect("t"),
    );
    // The running server was started on another address by a flag; the file never names it.
    let mut cfg = gitterrace_config::Config::default();
    cfg.server.addr = "127.0.0.1:9".to_owned();
    let mut env = Env::new(cfg);
    env.settings_dir = Some(dir.path().to_path_buf());
    let app = crate::build_server(vec![repo(root.path())], None, None, &env, None, None)
        .expect("router")
        .0;
    let token = token_of(&app);
    let mut req = Req::new("POST", "/_settings/check", Some(&token));
    req.body = serde_json::json!({
        "f": "config",
        "changes": [{ "id": "watch.interval_ms", "value": "750" }]
    })
    .to_string();
    let (status, reply) = send(&app, req).await;
    assert_eq!(status, 200, "{reply}");
    let v: serde_json::Value = serde_json::from_str(&reply).expect("json");
    assert_eq!(v["restart"], serde_json::json!([]), "{reply}");
}
