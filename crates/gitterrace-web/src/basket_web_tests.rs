//! The basket's page routes: add, list with anchors, remove, and refuse every write that lacks
//! the owner proof. Notes are plain text and the page escapes them.

use std::path::Path;

use axum::{body::Body, http::Request};
use gitterrace_testkit::Fixture;

use crate::app::Env;

fn server(root: &Path) -> axum::Router {
    let p = root.join("r");
    let f = Fixture::init(&p).write("a.txt", "a\nb\nc\n").commit("feat: one");
    crate::build_server(
        vec![f.path().to_path_buf()],
        None,
        None,
        &Env::new(gitterrace_config::Config::default()),
        None,
        None,
    )
    .expect("router")
    .0
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

const NOTE: &str = concat!(
    r#"{"kind":"line","rev":"HEAD","path":"a.txt","lines":{"start":2,"end":3},"#,
    r#""note":"<img src=x onerror=alert(1)>"}"#
);

#[tokio::test(flavor = "multi_thread")]
async fn a_finding_is_added_listed_with_its_status_and_removed() {
    let tmp = tempfile::tempdir().expect("tmp");
    let app = server(tmp.path());
    let (s, body) = call(&app, "POST", "/r/r/_findings", NOTE, true).await;
    assert_eq!(s, 200, "{body}");
    let (_, n) = call(&app, "GET", "/r/r/_findings?count=1", "", false).await;
    assert!(n.contains("\"count\":1"), "{n}");
    let (s, list) = call(&app, "GET", "/r/r/_findings", "", false).await;
    assert_eq!(s, 200);
    assert!(list.contains("\"status\":\"untouched\""), "{list}");
    let (s, gone) = call(&app, "DELETE", "/r/r/_findings", r#"{"ids":[]}"#, true).await;
    assert_eq!((s, gone.contains("\"removed\":1")), (200, true), "{gone}");
    let (_, n) = call(&app, "GET", "/r/r/_findings?count=1", "", false).await;
    assert!(n.contains("\"count\":0"), "{n}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_basket_writes_refuse_a_request_without_the_owner_proof() {
    let tmp = tempfile::tempdir().expect("tmp");
    let app = server(tmp.path());
    assert_eq!(call(&app, "POST", "/r/r/_findings", NOTE, false).await.0, 403);
    assert_eq!(call(&app, "DELETE", "/r/r/_findings", "{}", false).await.0, 403);
    assert_eq!(call(&app, "POST", "/r/r/_findings", "not json", true).await.0, 400);
    let (_, n) = call(&app, "GET", "/r/r/_findings?count=1", "", false).await;
    assert!(n.contains("\"count\":0"), "nothing was written: {n}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_with_markup_never_reaches_a_page_unescaped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let app = server(tmp.path());
    assert_eq!(call(&app, "POST", "/r/r/_findings", NOTE, true).await.0, 200);
    for uri in ["/r/r/", "/r/r/blob/HEAD/a.txt", "/r/r/worktree", "/r/r/log/HEAD"] {
        let (_, html) = call(&app, "GET", uri, "", false).await;
        assert!(!html.contains("<img src=x onerror"), "{uri} renders a note unescaped");
        assert!(html.contains("id=\"basketbtn\""), "{uri} lacks the basket button");
    }
}

/// The page, the JSON API and the basket agree on the switch: with `worktree_enabled` on the
/// anchor sees the edit made only in the working tree, with it off (file or this repository)
/// both answer as an unknown route and no anchor is derived.
#[tokio::test(flavor = "multi_thread")]
async fn the_worktree_switch_gates_the_basket_on_the_page_and_the_api() {
    for (off, cfg) in [
        (false, ""),
        (true, "[features]\nworktree_enabled = false\n"),
        (true, "[repo.r.features]\nworktree_enabled = false\n"),
    ] {
        let tmp = tempfile::tempdir().expect("tmp");
        let p = tmp.path().join("r");
        let f = Fixture::init(&p).write("a.txt", "a\nb\nc\n").commit("feat: one");
        let config = gitterrace_config::parse(cfg).expect("parse").config;
        let app = crate::build_server(
            vec![f.path().to_path_buf()],
            None,
            None,
            &Env::new(config),
            None,
            None,
        )
        .expect("router")
        .0;
        // Added straight into the cache: the page route is itself switched off here.
        let state = gitterrace_api::ApiState::open(&p).expect("open");
        let new = gitterrace_types::api::NewFinding {
            kind: "file".into(),
            lines: None,
            note: "n".into(),
            path: Some("a.txt".into()),
            query: None,
            rev: "HEAD".into(),
            sha: None,
        };
        state.finding_add(&new).expect("add");
        std::fs::write(p.join("a.txt"), "changed\n").expect("write");
        for uri in ["/r/r/_findings", "/r/r/api/v1/findings"] {
            let (s, body) = call(&app, "GET", uri, "", false).await;
            assert_eq!(s == 404, off, "{cfg:?} {uri}: {s} {body}");
            assert_eq!(body.contains("file changed"), !off, "{cfg:?} {uri}: {body}");
        }
    }
}

#[test]
fn the_panel_send_limit_is_the_payload_limit() {
    let js = include_str!("../static/basket.js");
    let want = format!("var SEND_MAX = {};", gitterrace_actions::MAX_REVIEW_FINDINGS);
    assert!(js.contains(&want), "basket.js must carry `{want}`");
}
