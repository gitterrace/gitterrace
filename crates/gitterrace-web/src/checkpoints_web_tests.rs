//! The checkpoint flow of the working tree page: take one, edit, see only the edit, keep viewed
//! marks, and refuse every write that lacks the owner proof.

use std::path::Path;

use axum::{body::Body, http::Request};
use gitterrace_testkit::Fixture;

use crate::app::Env;

fn server(root: &Path) -> (axum::Router, std::path::PathBuf) {
    let p = root.join("r");
    let f = Fixture::init(&p).write("a.txt", "a\n").write("b.txt", "b\n").commit("feat: one");
    let repo = f.path().to_path_buf();
    let router = crate::build_server(
        vec![repo.clone()],
        None,
        None,
        &Env::new(gitterrace_config::Config::default()),
        None,
        None,
    )
    .expect("router")
    .0;
    (router, repo)
}

async fn call(r: &axum::Router, method: &str, uri: &str, body: &str, owner: bool) -> (u16, String) {
    let host = gitterrace_testkit::on_port("localhost");
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", &host)
        .header("content-type", "application/json")
        .header("sec-fetch-site", "same-origin")
        .header("origin", format!("http://{host}"));
    if owner {
        b = b.header("cookie", crate::owner::test_cookie());
    }
    gitterrace_testkit::send(r, b.body(Body::from(body.to_owned())).expect("req")).await
}

fn field<'a>(json: &'a str, key: &str) -> &'a str {
    json.split(&format!("\"{key}\":\"")).nth(1).and_then(|s| s.split('"').next()).unwrap_or("")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_makes_the_since_tab_show_only_later_edits() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (app, repo) = server(tmp.path());
    std::fs::write(repo.join("a.txt"), "a\nedited\n").expect("w");
    let (s, body) = call(&app, "POST", "/r/r/_checkpoint", r#"{"scope":"all"}"#, true).await;
    assert_eq!(s, 200, "{body}");
    let id = field(&body, "id").to_owned();
    assert!(id.starts_with("cp-"), "{body}");
    std::fs::write(repo.join("b.txt"), "b\nlater\n").expect("w");
    let (_, plain) = call(&app, "GET", "/r/r/worktree", "", false).await;
    assert!(plain.contains("Since checkpoint"), "the tab is offered");
    assert!(plain.contains("data-checkpoint"), "the button is on the page");
    assert!(plain.contains("a.txt") && plain.contains("b.txt"));
    let (s, since) = call(&app, "GET", &format!("/r/r/worktree?since={id}"), "", false).await;
    assert_eq!(s, 200);
    assert!(since.contains("data-path=\"b.txt\""), "the later edit shows");
    assert!(!since.contains("data-path=\"a.txt\""), "the checkpointed edit does not");
    assert!(since.contains("1 file</summary>") || since.contains("unchanged since"), "{since}");
    assert!(since.contains("/r/r/worktree/diff/") || !since.contains("data-load-diff"));
    let (s, _) = call(&app, "GET", "/r/r/worktree?since=cp-000000000000", "", false).await;
    assert_eq!(s, 404, "an unknown checkpoint");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_viewed_mark_is_kept_by_the_server_per_patch() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (app, repo) = server(tmp.path());
    std::fs::write(repo.join("a.txt"), "a\nedited\n").expect("w");
    let (_, page) = call(&app, "GET", "/r/r/worktree", "", false).await;
    let patch = page.split("data-p=\"").nth(1).and_then(|s| s.split('"').next()).expect("data-p");
    assert_eq!(patch.len(), 16, "{patch}");
    let mark = format!(r#"{{"path":"a.txt","patch":"{patch}","viewed":true}}"#);
    assert_eq!(call(&app, "POST", "/r/r/_viewed", &mark, true).await.0, 200);
    let (_, list) = call(&app, "GET", "/r/r/api/v1/viewed", "", false).await;
    assert!(list.contains(patch) && list.contains("a.txt"), "{list}");
    let all = r#"{"items":[{"path":"b.txt","patch":"x"}],"checkpoint":true}"#;
    let (s, body) = call(&app, "POST", "/r/r/_viewed/all", all, true).await;
    assert_eq!(s, 200, "{body}");
    assert!(body.contains("\"checkpoint\":null"), "off unless review.has_auto_checkpoint: {body}");
    let off = format!(r#"{{"path":"a.txt","patch":"{patch}","viewed":false}}"#);
    assert_eq!(call(&app, "POST", "/r/r/_viewed", &off, true).await.0, 200);
    let (_, list) = call(&app, "GET", "/r/r/api/v1/viewed", "", false).await;
    assert!(!list.contains("a.txt"), "{list}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_writes_refuse_a_request_without_the_owner_proof() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (app, _) = server(tmp.path());
    for (uri, body) in [
        ("/r/r/_checkpoint", r#"{"scope":"all"}"#),
        ("/r/r/_viewed", r#"{"path":"a","patch":"b","viewed":true}"#),
        ("/r/r/_viewed/all", r#"{"items":[]}"#),
    ] {
        assert_eq!(call(&app, "POST", uri, body, false).await.0, 403, "{uri}");
        assert_eq!(call(&app, "POST", uri, "not json", true).await.0, 400, "{uri} bad body");
    }
    let (_, list) = call(&app, "GET", "/r/r/api/v1/checkpoints", "", false).await;
    assert!(list.contains("\"checkpoints\":[]"), "nothing was written: {list}");
}
