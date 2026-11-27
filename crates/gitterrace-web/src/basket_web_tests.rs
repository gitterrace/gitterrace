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

#[test]
fn the_note_box_limit_is_the_stored_note_limit() {
    let js = include_str!("../static/basket.js");
    let want = format!("var NOTE_MAX = {};", gitterrace_index::MAX_NOTE);
    assert!(js.contains(&want), "basket.js must carry `{want}`");
    assert!(!js.contains("maxLength"), "a UTF-16 maxlength disagrees with the byte limit");
}

/// Class guard: choosing rows is local state. A handler that only changes the ticked set must not
/// fetch the basket again, since that endpoint recomputes every anchor. It also catches any
/// later handler that calls `load()` straight after editing `picked`.
#[test]
fn selecting_a_batch_does_not_reload_the_basket() {
    let js = include_str!("../static/basket.js");
    let at = js.find("picked.clear();").expect("the select-first handler");
    // The handler's own close, at the button call's indent: inner callbacks close deeper.
    let handler = &js[at..js[at..].find("\n    });").map_or(js.len(), |e| at + e)];
    assert!(!handler.contains("load("), "select first must not call load(): {handler}");
    assert!(handler.contains("sync();"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_marks_the_lines_of_an_open_finding_and_links_the_note() {
    let tmp = tempfile::tempdir().expect("tmp");
    let app = server(tmp.path());
    assert_eq!(call(&app, "POST", "/r/r/_findings", NOTE, true).await.0, 200);
    let (s, html) = call(&app, "GET", "/r/r/blob/HEAD/a.txt", "", false).await;
    assert_eq!(s, 200);
    assert_eq!(html.matches("class=\"l fm\"").count(), 2, "lines 2 and 3 are marked: {html}");
    assert_eq!(html.matches("class=\"fm-link\"").count(), 1, "one link per finding");
    assert!(html.contains("href=\"#L2\" data-finding=\"f-"), "{html}");
    assert!(html.contains("title=\"&#60;img src=x onerror=alert(1)&#62;\""), "note is escaped");
    assert!(!html.contains("<img src=x onerror"), "{html}");
}

/// A public server never reads the basket, so a blob carries no marker and no note, even when
/// the cache holds a finding on its lines.
#[tokio::test(flavor = "multi_thread")]
async fn a_public_server_shows_no_finding_marker_on_a_blob() {
    const HOST: &str = "git.example.com";
    let tmp = tempfile::tempdir().expect("tmp");
    let p = tmp.path().join("r");
    let f = Fixture::init(&p).write("a.txt", "a\nb\nc\n").commit("feat: one");
    let new = gitterrace_types::api::NewFinding {
        kind: "line".into(),
        lines: Some(gitterrace_types::api::FindingLines { end: 3, start: 2 }),
        note: "secret note".into(),
        path: Some("a.txt".into()),
        query: None,
        rev: "HEAD".into(),
        sha: None,
    };
    gitterrace_api::ApiState::open(&p).expect("open").finding_add(&new).expect("add");
    let cfg = format!("[server]\nis_public = true\nhosts = [\"{HOST}\"]\n");
    let cfg = gitterrace_config::parse(&cfg).expect("parse").config;
    let app = crate::app_router_with(f.path(), cfg).expect("router");
    let req =
        Request::get("/r/r/blob/HEAD/a.txt").header("host", HOST).body(Body::empty()).expect("req");
    let (s, html) = gitterrace_testkit::send(&app, req).await;
    assert_eq!(s, 200, "{html}");
    assert!(html.contains("id=\"L2\""), "the blob rendered");
    assert!(!html.contains("fm-link") && !html.contains("l fm") && !html.contains("secret note"));
}
