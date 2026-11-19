use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use gitterrace_types::api::{
    BlameResponse, BlobResponse, CommitResponse, ContributorsResponse, EndpointDoc, IndexResponse,
    InsightList, InsightRow, LogResponse, ParamDoc, RefsResponse, RepoResponse, TreeResponse,
};
use serde::{Deserialize, Serialize};

use crate::core::ApiState;
use crate::endpoints::{self, ENDPOINTS};
use crate::error::ApiError;
use crate::insights;

type Reply<T> = Result<Json<T>, ApiError>;

/// The read-only JSON API. Nest it under `/api/v1`. Routes come from [`ENDPOINTS`], so an
/// endpoint cannot exist without its documentation, and each refuses a parameter it does not
/// list.
pub fn router(state: ApiState) -> Router {
    ENDPOINTS
        .iter()
        .fold(Router::new(), |r, e| {
            let params = e.params;
            let route = (e.route)().layer(middleware::from_fn(move |req, next| {
                known_params(params, req, next)
            }));
            r.route(e.path, route)
        })
        .layer(middleware::from_fn_with_state(state.clone(), etag))
        .with_state(state)
}

/// Runs a blocking read off the async runtime, holding one of the state's job slots until
/// the read exits.
async fn blocking<T, F>(state: &ApiState, f: F) -> Reply<T>
where
    T: Serialize + Send + 'static,
    F: FnOnce(&ApiState) -> Result<T, ApiError> + Send + 'static,
{
    let state = state.clone();
    let slot = state
        .job_slot()
        .await
        .ok_or_else(|| ApiError::Task("blocking job slots closed".to_owned()))?;
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        f(&state)
    })
    .await
    .map_err(|e| ApiError::Task(e.to_string()))?
    .map(Json)
}

/// Refuses a query key the endpoint does not list with 400: an unread key would answer
/// unfiltered numbers to a caller who thinks they are filtered.
async fn known_params(
    params: &'static [endpoints::Param],
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let unknown = req.uri().query().and_then(|q| {
        q.split('&')
            .map(|kv| kv.split_once('=').map_or(kv, |(k, _)| k))
            .find(|k| !k.is_empty() && !params.iter().any(|p| p.name == *k))
            .map(str::to_owned)
    });
    match unknown {
        Some(k) => {
            let known: Vec<&str> = params.iter().map(|p| p.name).collect();
            let known = if known.is_empty() {
                "none".to_owned()
            } else {
                known.join(", ")
            };
            ApiError::BadRequest(format!(
                "unknown parameter `{k}`. Known: {known}. Select commits with `q`"
            ))
            .into_response()
        }
        None => next.run(req).await,
    }
}

/// Answers `If-None-Match` with 304 and tags every response with [`ApiState::etag`].
async fn etag(
    State(state): State<ApiState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let slot = state.job_slot().await;
    let tag = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        state.etag()
    })
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

fn insight_rows() -> Vec<InsightRow> {
    insights::list()
        .map(|(name, summary)| InsightRow {
            name: name.to_owned(),
            summary: summary.to_owned(),
        })
        .collect()
}

pub(crate) async fn index() -> Json<IndexResponse> {
    Json(index_body(|_| true, true))
}

/// The `GET /` answer listing only the endpoints `keep` serves, and the insights only when
/// `insights` is set: a server with a feature off lists no endpoint of it.
pub fn index_body(keep: impl Fn(&str) -> bool, insights: bool) -> IndexResponse {
    let mut generator = endpoints::generator();
    let endpoints = endpoints::all()
        .iter()
        .filter(|e| keep(e.path))
        .map(|e| EndpointDoc {
            params: e
                .params
                .iter()
                .map(|p| ParamDoc {
                    description: p.description.to_owned(),
                    name: p.name.to_owned(),
                    required: p.required,
                    ty: p.ty.to_owned(),
                })
                .collect(),
            path: e.path.to_owned(),
            returns: e.returns(&mut generator),
            summary: e.summary.to_owned(),
        })
        .collect();
    let insights = if insights { insight_rows() } else { Vec::new() };
    IndexResponse {
        endpoints,
        insights,
        version: 1,
    }
}

pub(crate) async fn repo(State(s): State<ApiState>) -> Reply<RepoResponse> {
    blocking(&s, ApiState::repo_info).await
}

pub(crate) async fn refs(State(s): State<ApiState>) -> Reply<RefsResponse> {
    blocking(&s, ApiState::refs).await
}

#[derive(Deserialize)]
pub(crate) struct LogQuery {
    rev: Option<String>,
    path: Option<String>,
    page: Option<usize>,
    per_page: Option<usize>,
}

pub(crate) async fn log(
    State(s): State<ApiState>,
    Query(q): Query<LogQuery>,
) -> Reply<LogResponse> {
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
pub(crate) struct CommitQuery {
    /// Include hunks. Defaults to true.
    diff: Option<bool>,
    max_lines: Option<usize>,
}

pub(crate) async fn commit(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<CommitQuery>,
) -> Reply<CommitResponse> {
    blocking(&s, move |s| {
        s.commit(&id, q.diff.unwrap_or(true), q.max_lines)
    })
    .await
}

#[derive(Deserialize)]
pub(crate) struct TreeQuery {
    rev: Option<String>,
    path: Option<String>,
}

pub(crate) async fn tree(
    State(s): State<ApiState>,
    Query(q): Query<TreeQuery>,
) -> Reply<TreeResponse> {
    blocking(&s, move |s| s.tree(q.rev.as_deref(), q.path.as_deref())).await
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
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

pub(crate) async fn blob(
    State(s): State<ApiState>,
    Query(q): Query<FileQuery>,
) -> Reply<BlobResponse> {
    let path = need_path(&q)?;
    blocking(&s, move |s| {
        s.blob(q.rev.as_deref(), &path, q.start_line, q.limit)
    })
    .await
}

pub(crate) async fn blame(
    State(s): State<ApiState>,
    Query(q): Query<FileQuery>,
) -> Reply<BlameResponse> {
    let path = need_path(&q)?;
    blocking(&s, move |s| s.blame(q.rev.as_deref(), &path)).await
}

pub(crate) async fn contributors(
    State(s): State<ApiState>,
    Query(q): Query<InsightQuery>,
) -> Reply<ContributorsResponse> {
    let InsightQuery { rev, params } = q;
    blocking(&s, move |s| s.contributors(rev.as_deref(), &params)).await
}

pub(crate) async fn insight_list() -> Json<InsightList> {
    Json(InsightList {
        insights: insight_rows(),
    })
}

#[derive(Deserialize)]
pub(crate) struct InsightQuery {
    rev: Option<String>,
    #[serde(flatten)]
    params: insights::Params,
}

pub(crate) async fn insight(
    State(s): State<ApiState>,
    Path(name): Path<String>,
    Query(q): Query<InsightQuery>,
) -> Reply<insights::InsightResponse> {
    let InsightQuery { rev, params } = q;
    blocking(&s, move |s| s.insight(&name, rev.as_deref(), &params)).await
}
