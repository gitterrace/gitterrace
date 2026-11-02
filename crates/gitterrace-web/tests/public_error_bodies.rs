//! A public server whose insights cache is broken answers 5xx, and no body it sends, page or
//! API, names the folder the repository lives in.
#![expect(clippy::expect_used, reason = "test setup")]

use axum::body::Body;
use axum::http::Request;
use gitterrace_testkit::Fixture;
use http_body_util::BodyExt;
use tower::ServiceExt;

const HOST: &str = "git.example.com";
const MARKER: &str = "host-folder-marker";

async fn fetch(router: &axum::Router, uri: &str) -> (u16, String) {
    let req = Request::get(uri)
        .header("host", HOST)
        .body(Body::empty())
        .expect("req");
    let res = router.clone().oneshot(req).await.expect("res");
    let s = res.status().as_u16();
    let b = res.into_body().collect().await.expect("body").to_bytes();
    (s, String::from_utf8_lossy(&b).into_owned())
}

fn links(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for attr in ["href=\"", "src=\""] {
        for (i, _) in body.match_indices(attr) {
            let rest = &body[i + attr.len()..];
            let v = &rest[..rest.find('"').unwrap_or(0)];
            if v.starts_with('/') && !v.starts_with("//") {
                out.push(v.replace("&amp;", "&"));
            }
        }
    }
    out
}

#[tokio::test]
async fn public_error_bodies_never_name_host_folders() {
    let dir = tempfile::tempdir().expect("tmp");
    // The marker is a parent folder: the repository's own folder name is its public name.
    let p = Fixture::standard(&dir.path().join(MARKER).join("repo"))
        .path()
        .to_owned();
    let mut cfg = gitterrace_config::Config::default();
    cfg.server.public = true;
    cfg.server.hosts = vec![HOST.to_owned()];
    let router = gitterrace_web::app_router_with(&p, cfg).expect("router");
    // A directory where the cache expects a file makes every cache read fail, and the error
    // text of that failure names the full path.
    let commits = p.join(".git/gitterrace/commits.jsonl");
    std::fs::remove_file(&commits).ok();
    std::fs::create_dir_all(&commits).expect("mkdir");
    let mut todo: Vec<String> = ["/", "/stats", "/api/v1/"]
        .into_iter()
        .map(str::to_owned)
        .chain(gitterrace_api::ENDPOINTS.iter().map(|e| {
            format!(
                "/api/v1{}?path=src/a.rs",
                e.path.replace("{id}", "HEAD").replace("{name}", "summary")
            )
        }))
        .collect();
    let mut seen = std::collections::HashSet::new();
    let mut bad = Vec::new();
    let mut server_errors = 0;
    while let Some(u) = todo.pop() {
        if !seen.insert(u.clone()) || seen.len() > 200 {
            continue;
        }
        let (s, body) = fetch(&router, &u).await;
        if s >= 500 {
            server_errors += 1;
        }
        if s == 200 {
            todo.extend(
                links(&body)
                    .into_iter()
                    .map(|l| l.split('#').next().unwrap_or_default().to_owned()),
            );
        }
        if body.contains(MARKER) {
            bad.push(format!("{u} ({s})"));
        }
    }
    assert!(
        server_errors > 0,
        "the broken cache produced no 5xx: the guard tests nothing"
    );
    assert!(
        bad.is_empty(),
        "bodies name the host folder:\n{}",
        bad.join("\n")
    );
}
