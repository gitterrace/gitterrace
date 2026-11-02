use std::{path::Path, time::Duration};

use axum::{Router, body::Body, http::Request};
use gitterrace_testkit::Fixture;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::add::TOKEN_HEADER;

fn repo(dir: &Path, msg: &str) -> std::path::PathBuf {
    Fixture::init(dir)
        .write("src/a.rs", "fn a() {}\n")
        .commit(msg)
        .path()
        .canonicalize()
        .expect("canon")
}

async fn send(r: &Router, req: axum::http::request::Builder, body: String) -> (u16, String) {
    let res = r
        .clone()
        .oneshot(req.body(Body::from(body)).expect("req"))
        .await
        .expect("res");
    let s = res.status().as_u16();
    let b = res.into_body().collect().await.expect("body").to_bytes();
    (s, String::from_utf8_lossy(&b).into_owned())
}

fn req(method: &str, token: Option<&str>) -> axum::http::request::Builder {
    let b = Request::builder()
        .method(method)
        .uri("/_repos")
        .header("host", gitterrace_testkit::on_port("localhost"))
        .header("content-type", "application/json");
    match token {
        Some(t) => b.header(TOKEN_HEADER, t),
        None => b,
    }
}

fn named(n: &str) -> String {
    serde_json::json!({ "name": n }).to_string()
}

async fn get(r: &Router, uri: &str) -> (u16, String) {
    let b = Request::get(uri).header("host", gitterrace_testkit::on_port("localhost"));
    send(r, b, String::new()).await
}

fn server(paths: Vec<std::path::PathBuf>, cfg: gitterrace_config::Config) -> (Router, String) {
    let token = gitterrace_actions::Token::generate().expect("token");
    let secret = token.as_str().to_owned();
    let env = crate::app::Env::new(cfg);
    let (r, _) = crate::build_server(paths, None, None, &env, Some(token), None).expect("r");
    (r, secret)
}

/// Every file under `dir` with its length and modification time, to tell whether anything
/// still writes there.
fn snapshot(dir: &Path) -> Vec<(std::path::PathBuf, u64, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                todo.push(e.path());
            } else {
                out.push((e.path(), m.len(), m.modified().ok()));
            }
        }
    }
    out.sort();
    out
}

/// Guard class: taking a repository off needs the same gate as adding one (secret, no
/// browser), the last one stays, and afterwards it is gone from every listing and URL while
/// the others keep theirs. Re-adding brings it back under its name.
#[tokio::test]
async fn remove_is_gated_and_takes_the_repository_off() {
    let tmp = tempfile::tempdir().expect("tmp");
    let a = repo(&tmp.path().join("a"), "a root");
    let b = repo(&tmp.path().join("b"), "b root");
    let (r, secret) = server(vec![a, b.clone()], gitterrace_config::Config::default());
    let tok = Some(secret.as_str());
    assert_eq!(get(&r, "/r/b/").await.0, 200);

    assert_eq!(
        send(&r, req("DELETE", None), named("b")).await.0,
        403,
        "no proof"
    );
    assert_eq!(
        send(&r, req("DELETE", Some("x")), named("b")).await.0,
        403,
        "bad"
    );
    let browser = req("DELETE", tok).header("sec-fetch-site", "same-origin");
    assert_eq!(send(&r, browser, named("b")).await.0, 403, "browser");
    assert_eq!(get(&r, "/r/b/").await.0, 200, "still served");
    assert_eq!(
        send(&r, req("DELETE", tok), named("zz")).await.0,
        404,
        "unknown"
    );
    assert_eq!(send(&r, req("DELETE", tok), "{}".into()).await.0, 400);

    let by_path = serde_json::json!({ "path": b }).to_string();
    let (s, js) = send(&r, req("DELETE", tok), by_path).await;
    assert_eq!((s, js.contains("\"b\"")), (200, true), "{js}");
    let (s, page) = get(&r, "/r/b/").await;
    assert_eq!(s, 404, "a removed repository is a plain 404");
    assert!(page.contains("href=/"), "links to the overview: {page}");
    assert_eq!(get(&r, "/r/b/stats").await.0, 404);
    let (_, list) = get(&r, "/api/repos").await;
    assert!(!list.contains("/r/b/") && list.contains("/r/a/"), "{list}");
    assert_eq!(get(&r, "/r/a/").await.0, 200, "the other stays");
    assert_eq!(get(&r, "/").await.0, 302, "one left: / forwards to it");

    let (s, js) = send(&r, req("DELETE", tok), named("a")).await;
    assert_eq!(s, 409, "the last one stays: {js}");
    assert_eq!(get(&r, "/r/a/").await.0, 200);

    let add = serde_json::json!({ "path": b }).to_string();
    let (s, js) = send(&r, req("POST", tok), add).await;
    assert_eq!(s, 201, "{js}");
    assert!(js.contains("\"/r/b/\""), "same name again: {js}");
    assert_eq!(get(&r, "/r/b/").await.0, 200, "re-added");
}

/// Guard class: a public server has no way to change its repositories, with or without the
/// secret.
#[tokio::test]
async fn a_public_server_refuses_remove() {
    let tmp = tempfile::tempdir().expect("tmp");
    let a = repo(&tmp.path().join("a"), "a root");
    let b = repo(&tmp.path().join("b"), "b root");
    let mut cfg = gitterrace_config::Config::default();
    cfg.server.public = true;
    let (r, secret) = server(vec![a, b], cfg);
    let (s, _) = send(&r, req("DELETE", Some(&secret)), named("b")).await;
    assert!(s == 404 || s == 405, "public: {s}");
    // The secret routes are not mounted at all: the GET challenge reaches the guard.
    assert_eq!(get(&r, "/_repos?nonce=n1").await.0, 404, "no challenge in public");
    assert_eq!(get(&r, "/r/b/").await.0, 200, "still served");
}

/// Guard class: a removed repository's background work stops, so nothing writes to its cache
/// folder once it is off, even when its history moves. Red when its refresher keeps running.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_removed_repository_stops_indexing() {
    let tmp = tempfile::tempdir().expect("tmp");
    let a = repo(&tmp.path().join("a"), "a root");
    let b = repo(&tmp.path().join("b"), "b root");
    let mut cfg = gitterrace_config::Config::default();
    cfg.watch.interval_ms = 50;
    let (r, secret) = server(vec![a, b.clone()], cfg);
    let cache = b.join(".git/gitterrace");
    // Let its first index build land.
    let end = std::time::Instant::now() + Duration::from_secs(20);
    while snapshot(&cache).is_empty() && std::time::Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!snapshot(&cache).is_empty(), "indexed while served");
    assert_eq!(
        send(&r, req("DELETE", Some(&secret)), named("b")).await.0,
        200
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = snapshot(&cache);
    for i in 0..3 {
        std::fs::write(b.join(format!("src/n{i}.rs")), "fn n() {}\n").expect("write");
        gitterrace_testkit::git(&b, &["add", "-A"]);
        gitterrace_testkit::git(&b, &["commit", "-q", "-m", &format!("after removal {i}")]);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(snapshot(&cache), before, "no writes after removal");
}
