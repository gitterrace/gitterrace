use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::ApiState;
use crate::error::ApiError;
use crate::insights::{self, INSIGHTS};

type Reply = Result<Json<Value>, ApiError>;

/// The read-only JSON API. Nest it under `/api/v1`.
pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/repo", get(repo))
        .route("/refs", get(refs))
        .route("/log", get(log))
        .route("/commits/{id}", get(commit))
        .route("/tree", get(tree))
        .route("/blob", get(blob))
        .route("/blame", get(blame))
        .route("/contributors", get(contributors))
        .route("/insights", get(insight_list))
        .route("/insights/{name}", get(insight))
        .layer(middleware::from_fn_with_state(state.clone(), etag))
        .with_state(state)
}

/// Runs a blocking read off the async runtime.
async fn blocking<F>(state: &ApiState, f: F) -> Reply
where
    F: FnOnce(&ApiState) -> Result<Value, ApiError> + Send + 'static,
{
    let state = state.clone();
    tokio::task::spawn_blocking(move || f(&state))
        .await
        .map_err(|e| ApiError::Task(e.to_string()))?
        .map(Json)
}

/// Answers `If-None-Match` with 304 and tags every response. The `ETag` is the head
/// commit plus the cache version, so any new commit invalidates all of it.
async fn etag(
    State(state): State<ApiState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let tag = tokio::task::spawn_blocking(move || state.etag())
        .await
        .ok()
        .flatten();
    let Some(tag) = tag else {
        return next.run(req).await;
    };
    let matches = req
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == tag));
    let mut res = if matches && req.method() == axum::http::Method::GET {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        next.run(req).await
    };
    if let Ok(v) = HeaderValue::from_str(&tag) {
        res.headers_mut().insert(header::ETAG, v);
    }
    res
}

async fn index() -> Json<Value> {
    let insights: Vec<Value> = INSIGHTS
        .iter()
        .map(|i| json!({ "name": i.name, "summary": i.summary }))
        .collect();
    Json(json!({
        "version": 1,
        "endpoints": ["/repo", "/refs", "/log", "/commits/{id}", "/tree", "/blob", "/blame", "/contributors", "/insights", "/insights/{name}"],
        "insights": insights,
    }))
}

async fn repo(State(s): State<ApiState>) -> Reply {
    blocking(&s, ApiState::repo_info).await
}

async fn refs(State(s): State<ApiState>) -> Reply {
    blocking(&s, ApiState::refs).await
}

#[derive(Deserialize)]
struct LogQuery {
    rev: Option<String>,
    path: Option<String>,
    page: Option<usize>,
    per_page: Option<usize>,
}

async fn log(State(s): State<ApiState>, Query(q): Query<LogQuery>) -> Reply {
    blocking(&s, move |s| {
        s.log(
            q.rev.as_deref(),
            q.path.as_deref(),
            q.page.unwrap_or(1),
            q.per_page.unwrap_or(50),
        )
    })
    .await
}

#[derive(Deserialize)]
struct CommitQuery {
    /// Include hunks. Defaults to true.
    diff: Option<bool>,
    max_lines: Option<usize>,
}

async fn commit(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<CommitQuery>,
) -> Reply {
    blocking(&s, move |s| {
        s.commit(&id, q.diff.unwrap_or(true), q.max_lines)
    })
    .await
}

#[derive(Deserialize)]
struct TreeQuery {
    rev: Option<String>,
    path: Option<String>,
}

async fn tree(State(s): State<ApiState>, Query(q): Query<TreeQuery>) -> Reply {
    blocking(&s, move |s| s.tree(q.rev.as_deref(), q.path.as_deref())).await
}

#[derive(Deserialize)]
struct FileQuery {
    rev: Option<String>,
    path: Option<String>,
    start_line: Option<usize>,
    limit: Option<usize>,
}

fn need_path(q: &FileQuery) -> Result<String, ApiError> {
    q.path
        .clone()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| ApiError::BadRequest("missing query parameter: path".into()))
}

async fn blob(State(s): State<ApiState>, Query(q): Query<FileQuery>) -> Reply {
    let path = need_path(&q)?;
    blocking(&s, move |s| {
        s.blob(q.rev.as_deref(), &path, q.start_line, q.limit)
    })
    .await
}

async fn blame(State(s): State<ApiState>, Query(q): Query<FileQuery>) -> Reply {
    let path = need_path(&q)?;
    blocking(&s, move |s| s.blame(q.rev.as_deref(), &path)).await
}

#[derive(Deserialize)]
struct RevQuery {
    rev: Option<String>,
}

async fn contributors(State(s): State<ApiState>, Query(q): Query<RevQuery>) -> Reply {
    blocking(&s, move |s| s.contributors(q.rev.as_deref())).await
}

async fn insight_list() -> Json<Value> {
    let list: Vec<Value> = INSIGHTS
        .iter()
        .map(|i| json!({ "name": i.name, "summary": i.summary }))
        .collect();
    Json(json!({ "insights": list }))
}

#[derive(Deserialize)]
struct InsightQuery {
    rev: Option<String>,
    granularity: Option<String>,
    label: Option<String>,
    since: Option<i64>,
    limit: Option<usize>,
}

async fn insight(
    State(s): State<ApiState>,
    Path(name): Path<String>,
    Query(q): Query<InsightQuery>,
) -> Reply {
    let params = insights::Params {
        granularity: q.granularity,
        label: q.label,
        since: q.since,
        limit: q.limit,
    };
    blocking(&s, move |s| s.insight(&name, q.rev.as_deref(), &params)).await
}
