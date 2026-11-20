//! The small JSON answers the page scripts read: file paths for the finder, refs for the
//! switcher and the last commit per entry of a folder.

use axum::{
    extract::{Path, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use gitterrace_git::RefKind;

use crate::routes::Shared;
use crate::{app::blocking, error::Result, fmt};

fn json<T: serde::Serialize>(v: &T) -> Response {
    match serde_json::to_vec(v) {
        Ok(b) => (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("application/json")),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            ],
            b,
        )
            .into_response(),
        Err(e) => crate::error::Error::Index(std::io::Error::other(e)).into_response(),
    }
}

/// All file paths at a revision, for the client-side finder.
pub(super) async fn files(State(app): Shared, Path(rev): Path<String>) -> Result<Response> {
    let paths = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        a.file_paths(&id)
    })
    .await?;
    Ok(json(&*paths))
}

#[derive(serde::Serialize)]
struct RefBody {
    name: String,
    kind: &'static str,
    when: crate::when::When,
}

/// Branches and tags for the switcher. Remotes are left out to keep the list short.
pub(super) async fn refs(State(app): Shared) -> Result<Response> {
    let list = blocking(&app, |a| {
        let now = fmt::now();
        Ok(a.repo
            .refs()?
            .into_iter()
            .filter(|r| r.kind != RefKind::Remote)
            .map(|r| RefBody {
                kind: if r.kind == RefKind::Tag { "tag" } else { "branch" },
                when: fmt::ago(r.time, now),
                name: r.name,
            })
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(json(&list))
}

#[derive(serde::Serialize)]
struct LastBody {
    name: String,
    id: String,
    subject: String,
    when: crate::when::When,
}

pub(super) async fn last_root(state: Shared, Path(rev): Path<String>) -> Result<Response> {
    last(state, Path((rev, String::new()))).await
}

/// Last commit per entry of a directory, filled into the tree after first paint.
pub(super) async fn last(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Response> {
    let list = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let now = fmt::now();
        let found = a.last_commits(&id, path.trim_matches('/'))?;
        Ok(found
            .iter()
            .map(|c| LastBody {
                name: c.name.clone(),
                id: c.id.clone(),
                subject: c.subject.clone(),
                when: fmt::ago(c.time, now),
            })
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(json(&list))
}
