//! The checkpoint flow of the working tree page: take one, edit, see only the edit, keep viewed
//! marks, and refuse every write that lacks the owner proof.

use std::path::Path;

use axum::{body::Body, http::Request};
use gitterrace_testkit::Fixture;

use crate::app::Env;

fn server(root: &Path) -> (axum::Router, std::path::PathBuf) {
    server_with(root, gitterrace_config::Config::default())
}

fn server_with(
    root: &Path,
    config: gitterrace_config::Config,
) -> (axum::Router, std::path::PathBuf) {
    let p = root.join("r");
    let f = Fixture::init(&p).write("a.txt", "a\n").write("b.txt", "b\n").commit("feat: one");
    let repo = f.path().to_path_buf();
    let router = crate::build_server(vec![repo.clone()], None, None, &Env::new(config), None, None)
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
async fn viewed_marks_are_read_and_saved_with_the_public_api_off() {
    let tmp = tempfile::tempdir().expect("tmp");
    let config =
        gitterrace_config::parse("[features]\napi_enabled = false\n").expect("parse").config;
    let (app, _) = server_with(tmp.path(), config);
    assert_eq!(call(&app, "GET", "/r/r/api/v1/viewed", "", false).await.0, 404, "API stays off");
    let mark = r#"{"path":"a.txt","patch":"p1","viewed":true}"#;
    assert_eq!(call(&app, "POST", "/r/r/_viewed", mark, true).await.0, 200);
    let (s, list) = call(&app, "GET", "/r/r/_viewed", "", false).await;
    assert_eq!(s, 200, "{list}");
    assert!(list.contains("a.txt") && list.contains("p1"), "{list}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_migration_never_overwrites_a_mark_that_already_exists() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (app, _) = server(tmp.path());
    let mark = r#"{"path":"a.txt","patch":"new","viewed":true}"#;
    assert_eq!(call(&app, "POST", "/r/r/_viewed", mark, true).await.0, 200);
    let old = r#"{"items":[{"path":"a.txt","patch":"old"},{"path":"b.txt","patch":"b1"}],
        "missing_only":true}"#;
    let (s, body) = call(&app, "POST", "/r/r/_viewed/all", old, true).await;
    assert_eq!(s, 200, "{body}");
    assert!(body.contains("\"marked\":1"), "{body}");
    let (_, list) = call(&app, "GET", "/r/r/_viewed", "", false).await;
    assert!(list.contains("new") && !list.contains("old") && list.contains("b1"), "{list}");
}

/// Two tabs: tab A's old-marks move is held, tab B marks then unmarks the file, then the move
/// lands. The unmark must survive: the server state after the move, read as a reload does.
#[tokio::test(flavor = "multi_thread")]
async fn a_late_migration_does_not_bring_back_a_newer_unmark() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (app, _) = server(tmp.path());
    for on in ["true", "false"] {
        let b = format!(r#"{{"path":"a.txt","patch":"p1","viewed":{on}}}"#);
        assert_eq!(call(&app, "POST", "/r/r/_viewed", &b, true).await.0, 200);
    }
    let old = r#"{"items":[{"path":"a.txt","patch":"p1"},{"path":"b.txt","patch":"b1"}],
        "missing_only":true}"#;
    let (s, body) = call(&app, "POST", "/r/r/_viewed/all", old, true).await;
    assert_eq!(s, 200, "{body}");
    assert!(body.contains("\"marked\":1"), "{body}");
    let (_, list) = call(&app, "GET", "/r/r/_viewed", "", false).await;
    assert!(!list.contains("a.txt") && list.contains("b1"), "{list}");
}

/// The newer mark case: tab B marked with a newer hash before the old move lands.
#[tokio::test(flavor = "multi_thread")]
async fn a_late_migration_keeps_a_newer_mark_after_an_unmark() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (app, _) = server(tmp.path());
    for (on, h) in [("true", "p1"), ("false", "p1"), ("true", "p2")] {
        let b = format!(r#"{{"path":"a.txt","patch":"{h}","viewed":{on}}}"#);
        assert_eq!(call(&app, "POST", "/r/r/_viewed", &b, true).await.0, 200);
    }
    let old = r#"{"items":[{"path":"a.txt","patch":"p1"}],"missing_only":true}"#;
    let (s, body) = call(&app, "POST", "/r/r/_viewed/all", old, true).await;
    assert_eq!(s, 200, "{body}");
    assert!(body.contains("\"marked\":0"), "{body}");
    let (_, list) = call(&app, "GET", "/r/r/_viewed", "", false).await;
    assert!(list.contains("p2") && !list.contains("p1"), "{list}");
}

/// Class guard: a `missing_only` request checks and writes in one index call, so no tab can
/// land a mark between a separate read and write. A read of `viewed()` in the handler fails here.
#[test]
fn the_missing_only_check_and_write_share_one_lock() {
    let src = include_str!("checkpoints_web.rs");
    let at = src.find("if missing_only").expect("branch");
    let branch = &src[at..src[at..].find("let id").map_or(src.len(), |i| at + i)];
    assert!(branch.contains("viewed_add_missing") && !branch.contains(".viewed()"), "{branch}");
}

/// Class guard: every write the script sends waits in one queue, so the server sees them in the
/// order the reader made them. A later write posted outside the queue (a stale save landing
/// after a newer click, or a second migration racing the first) fails here.
#[test]
fn every_viewed_write_in_the_script_goes_through_the_queue() {
    let js = include_str!("../static/diffs-viewed.js");
    let posts = js.matches("post(\"").count();
    let queued = js.matches("enqueue(function").count();
    assert!(posts >= 2 && queued >= posts, "{posts} posts, {queued} queued");
}

/// Class guard for the page script: it reads and writes only its own `/_viewed` routes, since
/// the public API may be off. Any later script line that fetches `/api/v1` fails here.
#[test]
fn the_viewed_script_uses_web_routes_only() {
    let js = include_str!("../static/diffs-viewed.js");
    assert!(!js.contains("/api/v1"), "reads must not need the public API");
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

/// The page names the limit when the scan was cut, and keeps quiet when the list is whole.
#[tokio::test(flavor = "multi_thread")]
async fn a_cut_scan_shows_the_omitted_files_notice_and_a_whole_one_does_not() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (app, repo) = server(tmp.path());
    for i in 0..3 {
        std::fs::write(repo.join(format!("n{i}.txt")), "x\n").expect("w");
    }
    let (_, whole) = call(&app, "GET", "/r/r/worktree", "", false).await;
    assert!(!whole.contains("data-files-omitted"), "a list under the limit is whole");

    let cfg = gitterrace_config::parse("[limits]\nworktree_files = 2\n").expect("parse").config;
    let router = crate::build_server(vec![repo.clone()], None, None, &Env::new(cfg), None, None)
        .expect("router")
        .0;
    let (_, cut) = call(&router, "GET", "/r/r/worktree", "", false).await;
    assert!(cut.contains("data-files-omitted") && cut.contains("limits.worktree_files"), "{cut}");
}
