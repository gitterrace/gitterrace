//! The review basket on the page: the reads and writes the basket panel and the "Add to review"
//! menu entry call. The API and MCP stay read-only: these routes are the only writers, and they
//! write only the cache folder (`.git/gitterrace`). The notes are plain text, escaped by the
//! page wherever they are shown.

use axum::{
    extract::{Query, Request, State},
    response::Response,
};
use gitterrace_types::api::NewFinding;
use serde::Deserialize;

use crate::{
    checkpoints_web::{bad, refused, reply, with_api},
    guard,
    routes::Shared,
};

/// Most bytes of a write body: a finding is a path, a range and a note of at most 2000 bytes,
/// and a clear names the ids the page holds. Not configurable: a bound on what a page may post.
const BODY_LIMIT: usize = 256 << 10;

#[derive(Deserialize)]
pub(crate) struct ReadQuery {
    /// `?count`: only how many entries there are, which the header shows on every page and so
    /// must not pay for the anchors.
    count: Option<String>,
}

/// `GET /_findings` answers the basket as `GET /api/v1/findings` does, with each finding's
/// anchor. `GET /_findings?count=1` answers `{"count"}` alone, read from the store.
pub(crate) async fn list(State(app): Shared, Query(q): Query<ReadQuery>) -> Response {
    if q.count.is_some() {
        let a = std::sync::Arc::clone(&app);
        let n = tokio::task::spawn_blocking(move || a.index.findings_list().map(|l| l.len())).await;
        return match n {
            Ok(Ok(n)) => reply(&serde_json::json!({ "count": n })),
            _ => reply(&serde_json::json!({ "count": 0 })),
        };
    }
    match with_api(&app, gitterrace_api::ApiState::findings).await {
        Ok(v) => reply(&serde_json::to_value(&v).unwrap_or_default()),
        Err(res) => *res,
    }
}

/// `POST /_findings` with a finding (kind, rev, sha, path, lines, query, note) answers
/// `{"id"}`. The server checks the commit and the path against the repository and cuts the
/// note.
pub(crate) async fn add(State(app): Shared, req: Request) -> Response {
    if let Some(no) = refused(&app, &req) {
        return no;
    }
    let new = match guard::json_body::<NewFinding>(req, BODY_LIMIT, bad).await {
        Ok(b) => b,
        Err(res) => return *res,
    };
    match with_api(&app, move |api| api.finding_add(&new)).await {
        Ok(id) => reply(&serde_json::json!({ "id": id })),
        Err(res) => *res,
    }
}

#[derive(Deserialize)]
struct Ids {
    #[serde(default)]
    ids: Vec<String>,
}

/// `DELETE /_findings` with `{"ids":[...]}` removes those entries, and with no ids empties the
/// basket. Answers `{"removed"}`.
pub(crate) async fn remove(State(app): Shared, req: Request) -> Response {
    if let Some(no) = refused(&app, &req) {
        return no;
    }
    let body = match guard::json_body::<Ids>(req, BODY_LIMIT, bad).await {
        Ok(b) => b,
        Err(res) => return *res,
    };
    match with_api(&app, move |api| api.findings_remove(&body.ids)).await {
        Ok(n) => reply(&serde_json::json!({ "removed": n })),
        Err(res) => *res,
    }
}
