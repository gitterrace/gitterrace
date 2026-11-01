//! `POST /_repos`: adds a repository to the running server, so `git terrace open` joins one
//! server instead of starting a second. It is the only route that makes the server read a path
//! it was not started with, so it answers only a local program run by the same user: loopback
//! binds only, no browser (any `Origin` or `Sec-Fetch-Site` is refused, and there is no CORS,
//! so a page cannot read a reply either), and a per-process secret the server writes to a 0600
//! file in its 0700 state folder. The path must already be canonical and the top folder of a
//! git repository; the name follows the startup rules.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use gitterrace_actions::Token;
use gitterrace_git::Repo;

use crate::multi::{Added, Table};

/// Header carrying the secret from the token file.
pub const TOKEN_HEADER: &str = "x-gitterrace-add-token";
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
    Router::new().route("/_repos", post(add)).with_state(adder)
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

fn reply(status: StatusCode, body: &serde_json::Value) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(body),
    )
        .into_response()
}

fn bad(why: &str) -> Response {
    reply(
        StatusCode::BAD_REQUEST,
        &serde_json::json!({ "error": why }),
    )
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
        Ok(Err(why)) => return bad(&why),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let served = match adder.table.add(root.clone(), &stem) {
        Ok(Added::Fresh(s)) => s,
        Ok(Added::Present { name, base }) => {
            return reply(
                StatusCode::CONFLICT,
                &serde_json::json!({ "name": name, "base": base, "error": "already served" }),
            );
        }
        Err(stem) => return bad(&format!("unusable repository name {stem:?}")),
    };
    // Build now: starts the index refresher and the warm-up, as a startup repository gets.
    if let Err(e) = adder.table.router_of(&served).await {
        tracing::warn!("opening added {}: {e}", served.name);
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
    reply(
        StatusCode::CREATED,
        &serde_json::json!({ "name": served.name, "base": format!("/r/{}/", served.name) }),
    )
}

/// The repository top folder `path` names, and its folder name. `path` must be absolute and
/// canonical (no symlink, `..` or `.` anywhere), so the folder checked is the folder served.
/// A folder inside a repository is not one: only its top folder is. A repository holding the
/// actions file is refused, as at startup.
fn checked_root(path: &Path, actions_file: Option<&Path>) -> Result<(PathBuf, String), String> {
    if !path.is_absolute() {
        return Err("path must be absolute".to_owned());
    }
    let canon = path
        .canonicalize()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if canon != path {
        return Err("path must be canonical: no symlinks, `.` or `..`".to_owned());
    }
    if !canon.is_dir() {
        return Err("not a folder".to_owned());
    }
    let repo = Repo::discover(&canon).map_err(|_| "not a git repository".to_owned())?;
    let root = repo.root();
    if root != canon {
        return Err(format!(
            "not the top folder of a git repository (that is {})",
            root.display()
        ));
    }
    if actions_file.is_some_and(|f| f.canonicalize().is_ok_and(|f| f.starts_with(&root))) {
        return Err("this repository holds actions.toml".to_owned());
    }
    Ok((root, repo.name()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use axum::{body::Body, http::Request};
    use gitterrace_testkit::Fixture;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::add::TOKEN_HEADER;

    const ADD_PATH: &str = "/_repos";

    /// A two-file repository whose root commit is `msg`.
    fn repo(dir: &Path, msg: &str) -> std::path::PathBuf {
        Fixture::init(dir)
            .write("src/a.rs", "fn a() {}\n")
            .commit(msg)
            .path()
            .canonicalize()
            .expect("canon")
    }

    async fn send(
        r: &axum::Router,
        req: axum::http::request::Builder,
        body: String,
    ) -> (u16, String) {
        let res = r
            .clone()
            .oneshot(req.body(Body::from(body)).expect("req"))
            .await
            .expect("res");
        let s = res.status().as_u16();
        let b = res.into_body().collect().await.expect("body").to_bytes();
        (s, String::from_utf8_lossy(&b).into_owned())
    }

    fn post(token: Option<&str>) -> axum::http::request::Builder {
        let b = Request::post(ADD_PATH)
            .header("host", gitterrace_testkit::on_port("localhost"))
            .header("content-type", "application/json");
        match token {
            Some(t) => b.header(TOKEN_HEADER, t),
            None => b,
        }
    }

    fn body(p: &Path) -> String {
        serde_json::json!({ "path": p }).to_string()
    }

    async fn get(r: &axum::Router, uri: &str) -> (u16, String) {
        send(
            r,
            Request::get(uri).header("host", gitterrace_testkit::on_port("localhost")),
            String::new(),
        )
        .await
    }

    /// Both server shapes: one repository (added ones beside `/`) and several.
    async fn adds_and_guards(several: bool) {
        let tmp = tempfile::tempdir().expect("tmp");
        let first = repo(&tmp.path().join("a"), "a root");
        let mut start = vec![first.clone()];
        if several {
            start.push(repo(&tmp.path().join("x"), "x root"));
        }
        // Same folder name as the first, so the suffix rule shows.
        let b = repo(&tmp.path().join("other/a"), "b root");
        let token = gitterrace_actions::Token::generate().expect("token");
        let secret = token.as_str().to_owned();
        let env = crate::app::Env::new(gitterrace_config::Config::default());
        let (r, _) = crate::build_server(start, None, None, &env, Some(token), None).expect("r");
        let tok = Some(secret.as_str());

        assert_eq!(send(&r, post(None), body(&b)).await.0, 403, "no token");
        assert_eq!(send(&r, post(Some("x")), body(&b)).await.0, 403, "bad");
        let browser = post(tok).header("origin", "http://evil.example");
        assert_eq!(send(&r, browser, body(&b)).await.0, 403, "origin");
        let same = post(tok).header("sec-fetch-site", "same-origin");
        assert_eq!(send(&r, same, body(&b)).await.0, 403, "fetch metadata");
        assert_eq!(get(&r, "/r/a-2/").await.0, 404, "nothing added yet");

        let plain = tmp.path().join("plain");
        std::fs::create_dir_all(&plain).expect("dir");
        let plain = plain.canonicalize().expect("canon");
        assert_eq!(send(&r, post(tok), body(&plain)).await.0, 400, "no repo");
        assert_eq!(
            send(&r, post(tok), body(&b.join("src"))).await.0,
            400,
            "inside one"
        );
        assert_eq!(
            send(&r, post(tok), body(Path::new("rel"))).await.0,
            400,
            "relative"
        );
        #[cfg(unix)]
        {
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&b, &link).expect("link");
            let link = tmp.path().canonicalize().expect("c").join("link");
            assert_eq!(send(&r, post(tok), body(&link)).await.0, 400, "symlink");
        }
        assert_eq!(
            send(&r, post(tok), body(&b.join("../a"))).await.0,
            400,
            "dot-dot"
        );

        let (s, js) = send(&r, post(tok), body(&b)).await;
        assert_eq!(s, 201, "{js}");
        assert!(js.contains("\"base\":\"/r/a-2/\""), "{js}");
        let (s, page) = get(&r, "/r/a-2/").await;
        assert_eq!(s, 200);
        assert!(page.contains("/r/a-2/tree/"), "links carry the prefix");
        assert_eq!(get(&r, "/r/a-2/blob/HEAD/src/a.rs").await.0, 200);
        let (_, list) = get(&r, "/api/repos").await;
        assert!(list.contains("\"/r/a-2/\""), "{list}");
        // A permalink to the added repository resolves.
        let repo = gitterrace_git::Repo::discover(&b).expect("repo");
        let go = format!(
            "/go/{}/{}",
            repo.id().expect("id"),
            repo.resolve("HEAD").expect("head")
        );
        assert_eq!(get(&r, &go).await.0, 302);

        let (s, js) = send(&r, post(tok), body(&b)).await;
        assert_eq!((s, js.contains("\"name\":\"a-2\"")), (409, true), "{js}");
        let (s, js) = send(&r, post(tok), body(&first)).await;
        assert_eq!(s, 409, "a startup repository: {js}");
        let base = if several { "/r/a/" } else { "/" };
        assert!(js.contains(&format!("\"base\":\"{base}\"")), "{js}");
        if !several {
            assert_eq!(get(&r, "/").await.0, 200, "/ stays the repository");
        }
    }

    #[tokio::test]
    async fn single_repository_server_adds_beside_root() {
        adds_and_guards(false).await;
    }

    #[tokio::test]
    async fn multi_repository_server_adds() {
        adds_and_guards(true).await;
    }

    #[tokio::test]
    async fn no_token_no_route() {
        let t = tempfile::tempdir().expect("tmp");
        let a = repo(&t.path().join("a"), "a root");
        let b = repo(&t.path().join("b"), "b root");
        let env = crate::app::Env::new(gitterrace_config::Config::default());
        let (r, _) = crate::build_server(vec![a], None, None, &env, None, None).expect("r");
        // Unmounted, so the request reaches no handler: the page router's 404 (or 405).
        let status = send(&r, post(Some("x")), body(&b)).await.0;
        assert!(
            status == 404 || status == 405,
            "added without a token route: {status}"
        );
    }
}
