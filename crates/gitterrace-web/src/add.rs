//! `POST /_repos`: adds a repository to the running server, so `git terrace open` joins one
//! server instead of starting a second. It is the only route that makes the server read a path
//! it was not started with, so it answers only a local program run by the same user: loopback
//! binds only, no browser (any `Origin` or `Sec-Fetch-Site` is refused, and there is no CORS,
//! so a page cannot read a reply either), and a per-process secret the server writes to a 0600
//! file in its 0700 state folder. The path must already be canonical and the top folder of a
//! git repository; the name follows the startup rules. `DELETE /_repos` takes one off again
//! (`git terrace remove`) under the same gate. There is no remove button: a page has no secret,
//! and on a shared screen anyone could press it.

use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use gitterrace_actions::Token;
use gitterrace_git::Repo;

use crate::multi::{Added, Kept, Table};

/// Why a path cannot be added. The text goes back to the caller of `POST /_repos`.
#[derive(Debug, thiserror::Error)]
enum RootError {
    #[error("path must be absolute")]
    NotAbsolute,
    #[error("cannot read {}: {source}", path.display())]
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("path must be canonical: no symlinks, `.` or `..`")]
    NotCanonical,
    #[error("not a folder")]
    NotFolder,
    #[error("not a git repository")]
    NotRepo,
    #[error("not the top folder of a git repository (that is {})", top.display())]
    NotTop { top: PathBuf },
    #[error("this repository holds actions.toml")]
    HoldsActions,
}

/// Every body `/_repos` answers with. Absent keys stay absent.
#[derive(serde::Serialize)]
struct AddReply<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof: Option<&'a str>,
}

impl<'a> AddReply<'a> {
    const NONE: Self = Self {
        base: None,
        error: None,
        name: None,
        proof: None,
    };

    const fn error(why: &'a str) -> Self {
        Self {
            error: Some(why),
            ..Self::NONE
        }
    }
}

/// Header carrying the secret from the token file.
pub const TOKEN_HEADER: &str = "x-gitterrace-add-token";
/// Not configurable: a request body bound against hostile pages posting to the local server.
const BODY_LIMIT: usize = 8 * 1024;

/// What the route needs: the table to grow, the permalink and action targets to extend, the
/// secret, and the actions file an added repository must not contain.
pub(crate) struct Adder {
    pub(crate) table: Arc<Table>,
    pub(crate) site: Arc<crate::actions::Site>,
    pub(crate) token: Token,
    pub(crate) actions_file: Option<PathBuf>,
    pub(crate) state: Option<PathBuf>,
}

#[derive(serde::Deserialize)]
struct AddRequest {
    path: PathBuf,
}

/// Mounted only on a loopback bind with a token; elsewhere the route does not exist.
pub(crate) fn router(adder: Arc<Adder>) -> Router {
    Router::new()
        .route("/_repos", get(prove).post(add).delete(remove))
        .with_state(adder)
}

#[derive(serde::Deserialize)]
struct Challenge {
    nonce: String,
}

/// `GET /_repos?nonce=N`: proves this server holds the secret in the token file, without
/// sending it, so `open` checks who listens before it sends a repository path. Browsers are
/// refused as on the add route.
async fn prove(
    State(adder): State<Arc<Adder>>,
    headers: HeaderMap,
    axum::extract::Query(c): axum::extract::Query<Challenge>,
) -> Response {
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return (
            StatusCode::FORBIDDEN,
            "forbidden: browsers cannot add repositories\n",
        )
            .into_response();
    }
    if c.nonce.is_empty() || c.nonce.len() > 128 {
        return bad("nonce must be 1 to 128 bytes");
    }
    reply(
        StatusCode::OK,
        &AddReply {
            proof: Some(&adder.token.prove(&c.nonce)),
            ..AddReply::NONE
        },
    )
}

/// Why a request is refused before its body is read. Any browser marker refuses: a fetch from
/// a page always carries `Sec-Fetch-Site` (and a cross-origin one `Origin`), a local program
/// sends neither.
fn refusal(h: &HeaderMap, token: &Token) -> Option<&'static str> {
    if h.contains_key(header::ORIGIN) || h.contains_key("sec-fetch-site") {
        return Some("forbidden: browsers cannot add repositories\n");
    }
    let offered = h.get(TOKEN_HEADER).and_then(|v| v.to_str().ok());
    if !offered.is_some_and(|t| token.verify(t)) {
        return Some("forbidden: bad token\n");
    }
    None
}

fn reply(status: StatusCode, body: &AddReply<'_>) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(body),
    )
        .into_response()
}

fn bad(why: &str) -> Response {
    reply(StatusCode::BAD_REQUEST, &AddReply::error(why))
}

async fn add(State(adder): State<Arc<Adder>>, req: Request) -> Response {
    if let Some(why) = refusal(req.headers(), &adder.token) {
        return (StatusCode::FORBIDDEN, why).into_response();
    }
    let Ok(bytes) = axum::body::to_bytes(req.into_body(), BODY_LIMIT).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "body too large\n").into_response();
    };
    let Ok(ar) = serde_json::from_slice::<AddRequest>(&bytes) else {
        return bad("malformed request");
    };
    let checked = tokio::task::spawn_blocking({
        let file = adder.actions_file.clone();
        move || checked_root(&ar.path, file.as_deref())
    })
    .await;
    let (root, stem) = match checked {
        Ok(Ok(found)) => found,
        Ok(Err(why)) => return bad(&why.to_string()),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let served = match adder.table.add(root.clone(), &stem) {
        Ok(Added::Fresh(s)) => s,
        Ok(Added::Present { name, base }) => {
            return reply(
                StatusCode::CONFLICT,
                &AddReply {
                    base: Some(&base),
                    error: Some("already served"),
                    name: Some(&name),
                    proof: None,
                },
            );
        }
        Err(e) => return bad(&e.to_string()),
    };
    // Build now: starts the index refresher and the warm-up, as a startup repository gets. A
    // repository that will not open is taken out again and never remembered, so it does not
    // come back on the next start. Only a local server takes additions, so the reason (which
    // may name a host folder) goes back to the caller.
    if let Err(e) = adder.table.router_of(&served).await {
        tracing::warn!("opening added {}: {e}", served.name);
        adder.table.remove(&served.name);
        return reply(
            StatusCode::UNPROCESSABLE_ENTITY,
            &AddReply::error(&format!("cannot open the repository: {e}")),
        );
    }
    match Repo::discover(&root) {
        Ok(repo) => adder.site.push_target(crate::actions::Target {
            key: served.name.clone(),
            base: format!("/r/{}", served.name),
            repo,
        }),
        Err(e) => tracing::warn!("permalinks to added {}: {e}", served.name),
    }
    let state = adder.state.clone();
    tokio::task::spawn_blocking(move || crate::remember(state.as_deref(), &[root]));
    tracing::info!("added {} at /r/{}/", stem, served.name);
    // Open pages show the switcher, or its new entry, without a reload.
    adder.table.announce();
    reply(
        StatusCode::CREATED,
        &AddReply {
            base: Some(&format!("/r/{}/", served.name)),
            name: Some(&served.name),
            ..AddReply::NONE
        },
    )
}

/// What `DELETE /_repos` names: a URL name, or a canonical top folder.
#[derive(serde::Deserialize)]
struct RemoveRequest {
    name: Option<String>,
    path: Option<PathBuf>,
}

/// `DELETE /_repos`: takes a repository off the running server. Its pages then answer 404, its
/// background tasks stop and open switchers drop it; the on-disk index and the name stay, so
/// adding it again is fast and keeps its URL. The last repository stays: a server serving
/// nothing has nothing at `/`, and stopping it is the user's call.
async fn remove(State(adder): State<Arc<Adder>>, req: Request) -> Response {
    if let Some(why) = refusal(req.headers(), &adder.token) {
        return (StatusCode::FORBIDDEN, why).into_response();
    }
    let Ok(bytes) = axum::body::to_bytes(req.into_body(), BODY_LIMIT).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "body too large\n").into_response();
    };
    let Ok(rr) = serde_json::from_slice::<RemoveRequest>(&bytes) else {
        return bad("malformed request");
    };
    if rr.name.is_none() && rr.path.is_none() {
        return bad("name or path required");
    }
    match adder.table.take(rr.name.as_deref(), rr.path.as_deref()) {
        Ok(name) => {
            adder.site.drop_target(&name);
            tracing::info!("removed {name}");
            adder.table.announce();
            reply(
                StatusCode::OK,
                &AddReply {
                    name: Some(&name),
                    ..AddReply::NONE
                },
            )
        }
        Err(Kept::NotServed) => reply(StatusCode::NOT_FOUND, &AddReply::error("not served")),
        Err(Kept::Last) => reply(
            StatusCode::CONFLICT,
            &AddReply::error("the last repository stays; stop the server instead"),
        ),
    }
}

/// The repository top folder `path` names, and its folder name. `path` must be absolute and
/// canonical (no symlink, `..` or `.` anywhere), so the folder checked is the folder served.
/// A folder inside a repository is not one: only its top folder is. A repository holding the
/// actions file is refused, as at startup.
fn checked_root(path: &Path, actions_file: Option<&Path>) -> Result<(PathBuf, String), RootError> {
    if !path.is_absolute() {
        return Err(RootError::NotAbsolute);
    }
    // Refused by name, before any resolving: a verbatim `\\?\` path on Windows keeps `..` as a
    // plain name, so comparing it with its canonical form let `x\..\a` through as canonical.
    if path
        .components()
        .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        return Err(RootError::NotCanonical);
    }
    let canon = path
        .canonicalize()
        .map_err(|source| RootError::Unreadable {
            path: path.to_owned(),
            source,
        })?;
    if canon != path {
        return Err(RootError::NotCanonical);
    }
    if !canon.is_dir() {
        return Err(RootError::NotFolder);
    }
    let repo = Repo::discover(&canon).map_err(|_| RootError::NotRepo)?;
    let root = repo.root();
    if root != canon {
        return Err(RootError::NotTop { top: root });
    }
    if actions_file.is_some_and(|f| f.canonicalize().is_ok_and(|f| f.starts_with(&root))) {
        return Err(RootError::HoldsActions);
    }
    Ok((root, repo.name()))
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn root_errors_keep_their_text() {
        assert_eq!(RootError::NotAbsolute.to_string(), "path must be absolute");
        assert_eq!(
            RootError::NotCanonical.to_string(),
            "path must be canonical: no symlinks, `.` or `..`"
        );
        assert_eq!(RootError::NotFolder.to_string(), "not a folder");
        assert_eq!(RootError::NotRepo.to_string(), "not a git repository");
        assert_eq!(
            RootError::NotTop { top: "/r".into() }.to_string(),
            "not the top folder of a git repository (that is /r)"
        );
        assert_eq!(
            RootError::HoldsActions.to_string(),
            "this repository holds actions.toml"
        );
    }

    #[test]
    fn replies_omit_absent_keys_and_sort_the_rest() {
        let body = serde_json::to_string(&AddReply {
            base: Some("/r/a/"),
            error: Some("already served"),
            name: Some("a"),
            proof: None,
        })
        .expect("json");
        assert_eq!(
            body,
            serde_json::json!({ "name": "a", "base": "/r/a/", "error": "already served" })
                .to_string()
        );
        let body = serde_json::to_string(&AddReply::error("x")).expect("json");
        assert_eq!(body, r#"{"error":"x"}"#);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "add/remove_tests.rs"]
mod remove_tests;
