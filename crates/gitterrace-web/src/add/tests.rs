use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use axum::{body::Body, http::Request};
use gitterrace_testkit::Fixture;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::add::TOKEN_HEADER;

const ADD_PATH: &str = "/_repos";

/// A two-file repository whose root commit is `msg`.
pub(super) fn repo(dir: &Path, msg: &str) -> std::path::PathBuf {
    Fixture::init(dir)
        .write("src/a.rs", "fn a() {}\n")
        .commit(msg)
        .path()
        .canonicalize()
        .expect("canon")
}

pub(super) async fn send(
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

/// Both server shapes: one repository and several.
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
    let start_len = start.len();
    let (r, _) = crate::build_server(start, None, None, &env, Some(token), None).expect("r");
    let tok = Some(secret.as_str());

    assert_eq!(send(&r, post(None), body(&b)).await.0, 403, "no token");
    assert_eq!(send(&r, post(Some("x")), body(&b)).await.0, 403, "bad");
    let browser = post(tok).header("origin", "http://evil.example");
    assert_eq!(send(&r, browser, body(&b)).await.0, 403, "origin");
    let same = post(tok).header("sec-fetch-site", "same-origin");
    assert_eq!(send(&r, same, body(&b)).await.0, 403, "fetch metadata");
    let (_, list) = get(&r, "/api/repos").await;
    assert_eq!(
        list.matches("\"href\"").count(),
        start_len,
        "nothing added yet"
    );

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
    // Spelled as text: on Windows `b` is a verbatim `\\?\` path, and `join("../a")` folds the
    // `..` there, which sent `b` itself and was rightly added.
    let dot_dot = PathBuf::from(format!(
        "{}{}..{}a",
        b.display(),
        MAIN_SEPARATOR,
        MAIN_SEPARATOR
    ));
    assert_eq!(send(&r, post(tok), body(&dot_dot)).await.0, 400, "dot-dot");

    let (s, js) = send(&r, post(tok), body(&b)).await;
    assert_eq!(s, 202, "{js}");
    // Same folder name as the first: the newcomer is named by its path, the first keeps `a`.
    let v: serde_json::Value = serde_json::from_str(&js).expect("json");
    let name = v["name"].as_str().expect("name").to_owned();
    assert!(name.starts_with("a-") && name.len() == 8, "{js}");
    assert_eq!(v["base"], format!("/r/{name}/"), "{js}");
    let (s, page) = get(&r, &format!("/r/{name}/")).await;
    assert_eq!(s, 200);
    assert!(
        page.contains(&format!("/r/{name}/tree/")),
        "links carry the prefix"
    );
    assert_eq!(
        get(&r, &format!("/r/{name}/blob/HEAD/src/a.rs")).await.0,
        200
    );
    assert_eq!(get(&r, "/r/a/").await.0, 200, "the first keeps its URL");
    let (_, list) = get(&r, "/api/repos").await;
    assert!(list.contains(&format!("\"/r/{name}/\"")), "{list}");
    // A permalink to the added repository resolves.
    let repo = gitterrace_git::Repo::discover(&b).expect("repo");
    let go = format!(
        "/go/{}/{}",
        repo.id().expect("id"),
        repo.resolve("HEAD").expect("head")
    );
    assert_eq!(get(&r, &go).await.0, 302);

    let (s, js) = send(&r, post(tok), body(&b)).await;
    assert_eq!(
        (s, js.contains(&format!("\"name\":\"{name}\""))),
        (409, true),
        "{js}"
    );
    let (s, js) = send(&r, post(tok), body(&first)).await;
    assert_eq!(s, 409, "a startup repository: {js}");
    assert!(js.contains("\"base\":\"/r/a/\""), "{js}");
    // `/` now indexes both, where it forwarded to the one before.
    assert_eq!(get(&r, "/").await.0, 200, "the index");
}

/// Guard class: the challenge proves the secret without revealing it, and a browser
/// cannot use it as an oracle.
#[tokio::test]
async fn the_challenge_proves_the_secret_without_sending_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let first = repo(&tmp.path().join("a"), "a root");
    let token = gitterrace_actions::Token::generate().expect("token");
    let secret = token.as_str().to_owned();
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let (r, _) = crate::build_server(vec![first], None, None, &env, Some(token), None).expect("r");
    let (s, js) = get(&r, "/_repos?nonce=n1").await;
    let want = gitterrace_actions::token_proof(&secret, "n1");
    assert_eq!(
        (s, js.contains(&want), js.contains(&secret)),
        (200, true, false),
        "{js}"
    );
    let browser = Request::get("/_repos?nonce=n1")
        .header("host", gitterrace_testkit::on_port("localhost"))
        .header("sec-fetch-site", "same-origin");
    assert_eq!(send(&r, browser, String::new()).await.0, 403);
    assert_eq!(get(&r, "/_repos?nonce=").await.0, 400);
}

#[tokio::test]
async fn single_repository_server_adds_beside_root() {
    adds_and_guards(false).await;
}

/// Guard class: an open page hears that the server's repositories changed, so a joined
/// repository shows in the switcher without a reload. Red when an add announces nothing.
#[tokio::test]
async fn an_open_page_hears_a_repository_join() {
    let tmp = tempfile::tempdir().expect("tmp");
    let a = repo(&tmp.path().join("a"), "a root");
    let b = repo(&tmp.path().join("b"), "b root");
    let token = gitterrace_actions::Token::generate().expect("token");
    let secret = token.as_str().to_owned();
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let (r, _) = crate::build_server(vec![a], None, None, &env, Some(token), None).expect("r");
    // A's page is open: its stream is up before B joins.
    assert_eq!(get(&r, "/r/a/").await.0, 200);
    let res = r
        .clone()
        .oneshot(
            Request::get("/r/a/events")
                .header("host", gitterrace_testkit::on_port("localhost"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("events");
    assert_eq!(res.status(), 200);
    let mut events = res.into_body();
    assert_eq!(send(&r, post(Some(&secret)), body(&b)).await.0, 202);
    let mut seen = String::new();
    let end = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while !seen.contains("\"repos\"") && tokio::time::Instant::now() < end {
        match tokio::time::timeout_at(end, events.frame()).await {
            Ok(Some(Ok(f))) => {
                if let Some(d) = f.data_ref() {
                    seen.push_str(&String::from_utf8_lossy(d));
                }
            }
            _ => break,
        }
    }
    assert!(seen.contains("\"repos\""), "no repos event: {seen}");
}

#[tokio::test]
async fn multi_repository_server_adds() {
    adds_and_guards(true).await;
}

/// A repository that is found but whose cache cannot be made answers 422 before anything is
/// added: it is not listed, not written to the state file, and a restart from that state does
/// not bring it back. Red when the check moves behind the 202.
#[cfg(unix)]
#[tokio::test]
async fn an_addition_that_will_not_open_is_refused_and_forgotten() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().expect("tmp");
    let a = repo(&t.path().join("a"), "a root");
    let b = repo(&t.path().join("b"), "b root");
    let git_dir = b.join(".git");
    std::fs::set_permissions(&git_dir, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    if std::fs::create_dir(git_dir.join("probe")).is_ok() {
        // Running as a user that ignores permissions: nothing to observe.
        return;
    }
    let state = t.path().join("state");
    let start = |state: &Path| {
        let token = gitterrace_actions::Token::generate().expect("token");
        let secret = token.as_str().to_owned();
        let env = crate::app::Env::new(gitterrace_config::Config::default());
        let (server, _) = crate::build_server(
            vec![a.clone()],
            None,
            None,
            &env,
            Some(token),
            Some(state.to_owned()),
        )
        .expect("r");
        (server, secret)
    };
    let (server, secret) = start(&state);
    let (status, js) = send(&server, post(Some(&secret)), body(&b)).await;
    std::fs::set_permissions(&git_dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    assert_eq!(status, 422, "{js}");
    assert!(js.contains("cannot open the repository"), "{js}");
    assert!(
        !get(&server, "/api/repos").await.1.contains("/r/b/"),
        "listed"
    );
    assert_eq!(get(&server, "/r/b/").await.0, 404, "not served");
    let recorded = std::fs::read_to_string(state.join("repos.json")).unwrap_or_default();
    assert!(!recorded.contains(b.to_str().expect("utf8")), "{recorded}");
    let (again, _) = start(&state);
    let (_, list) = get(&again, "/api/repos").await;
    assert!(!list.contains("/r/b/"), "brought back by a restart: {list}");
}

/// Guard class: any wait on the repository itself before the reply, whether its router, its
/// index, its cache or its git reads. Opening the added repository is held at its cache's
/// writer lock (an old-format cache resets under it), and the add still answers while the
/// lock is held; the repository is listed at once and opens once the lock goes.
#[tokio::test(flavor = "multi_thread")]
async fn an_addition_answers_before_the_repository_opens() {
    let t = tempfile::tempdir().expect("tmp");
    let a = repo(&t.path().join("a"), "a root");
    let b = repo(&t.path().join("b"), "b root");
    let cache = b.join(".git/gitterrace");
    std::fs::create_dir_all(&cache).expect("cache");
    std::fs::write(cache.join("VERSION"), "0\n").expect("old format");
    let lock = std::fs::File::create(cache.join("lock")).expect("lock");
    lock.lock().expect("hold the writer lock");
    let token = gitterrace_actions::Token::generate().expect("token");
    let secret = token.as_str().to_owned();
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let (r, _) = crate::build_server(vec![a], None, None, &env, Some(token), None).expect("r");
    let sent = {
        let (r, b) = (r.clone(), b.clone());
        tokio::spawn(async move { send(&r, post(Some(&secret)), body(&b)).await })
    };
    gitterrace_testkit::wait::until("the add answers while the lock is held", || async {
        sent.is_finished()
    })
    .await;
    let (status, js) = sent.await.expect("add");
    assert_eq!(status, 202, "{js}");
    assert!(js.contains("\"base\":\"/r/b/\""), "{js}");
    assert!(
        get(&r, "/api/repos").await.1.contains("/r/b/"),
        "listed at once"
    );
    lock.unlock().expect("release");
    ready(&r, "/r/b").await;
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

/// The hrefs of `html` that stay on this server.
fn local_links(html: &str) -> Vec<String> {
    html.split("href=\"")
        .skip(1)
        .filter_map(|r| r.split('"').next())
        .filter(|l| l.starts_with('/') && !l.starts_with("//"))
        .map(|l| l.replace("&amp;", "&"))
        .collect()
}

/// A server on `paths` that takes additions and keeps names in `state`.
fn server(paths: Vec<std::path::PathBuf>, state: &Path) -> (axum::Router, String) {
    let token = gitterrace_actions::Token::generate().expect("token");
    let secret = token.as_str().to_owned();
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let (r, _) = crate::build_server(paths, None, None, &env, Some(token), Some(state.to_owned()))
        .expect("r");
    (r, secret)
}

/// Guards (a), (b) and (d): links never change. Every link a lone repository's pages
/// render, and its `/go` permalink, answers the same after another repository joins and
/// after a restart with both in the other order. A link is under `/r/NAME` from the
/// start, never at `/`; the one exception is `/`, which only forwards.
#[tokio::test(flavor = "multi_thread")]
async fn links_handed_out_survive_a_join_and_a_restart() {
    let t = tempfile::tempdir().expect("tmp");
    let state = t.path().join("state");
    let a = repo(&t.path().join("a"), "a root");
    let b = repo(&t.path().join("b"), "b root");
    let id = gitterrace_git::Repo::discover(&a)
        .expect("repo")
        .id()
        .expect("id");
    let head = gitterrace_testkit::git(&a, &["rev-parse", "HEAD"]);
    let go = [
        format!("/go/{id}/{head}"),
        format!("/go/{id}/{head}/src/a.rs"),
    ];

    let (alone, secret) = server(vec![a.clone()], &state);
    ready(&alone, "/r/a").await;
    let mut urls: Vec<String> = Vec::new();
    for page in [
        "/r/a/",
        "/r/a/log/HEAD",
        "/r/a/refs",
        "/r/a/blob/HEAD/src/a.rs",
    ] {
        let (s, html) = get(&alone, page).await;
        assert_eq!(s, 200, "{page}");
        for l in local_links(&html) {
            // The guard on the class: no page of a lone repository links outside its
            // prefix except to the server's own routes, which never move.
            let server_route = l == "/" || l.starts_with("/static/") || l.starts_with("/go/");
            assert!(
                l.starts_with("/r/a/") || server_route,
                "{page} links {l} outside /r/a/"
            );
            if l.starts_with("/r/a/") && !urls.contains(&l) {
                urls.push(l);
            }
        }
    }
    assert!(urls.len() > 10, "too few links to guard: {urls:?}");
    let mut before = Vec::new();
    for u in &urls {
        before.push(get(&alone, u).await);
    }
    let mut gone = Vec::new();
    for g in &go {
        gone.push(location(&alone, g).await);
    }

    let (s, js) = send(&alone, post(Some(&secret)), body(&b)).await;
    assert_eq!(s, 202, "{js}");
    let same = |r: axum::Router, why: &'static str| {
        let (urls, before, gone, go) = (urls.clone(), before.clone(), gone.clone(), go.clone());
        async move {
            for (u, (s0, b0)) in urls.iter().zip(&before) {
                let (s1, b1) = get(&r, u).await;
                assert_eq!(s1, *s0, "{why}: {u}");
                // Same content: the main column; the header now has a switcher.
                assert_eq!(main_of(&b1), main_of(b0), "{why}: {u}");
            }
            for (g, l0) in go.iter().zip(&gone) {
                assert_eq!(&location(&r, g).await, l0, "{why}: {g}");
            }
        }
    };
    same(alone.clone(), "after b joined").await;
    let (again, _) = server(vec![b, a], &state);
    ready(&again, "/r/a").await;
    same(again, "restarted as b, a").await;
}

/// Waits until the repository at `prefix` has built its cache, so an insights page reads the
/// same before and after rather than "indexing" once.
async fn ready(r: &axum::Router, prefix: &str) {
    let uri = format!("{prefix}/api/progress");
    let ready = || async { get(r, &uri).await.1.contains("\"ready\":true") };
    gitterrace_testkit::wait::until(&format!("{prefix} is ready"), ready).await;
}

/// The `<main>` of a page: what it shows, without the header around it.
fn main_of(html: &str) -> &str {
    html.split_once("<main")
        .map_or(html, |(_, m)| m)
        .split_once("</main>")
        .map_or("", |(m, _)| m)
}

async fn location(r: &axum::Router, uri: &str) -> Option<String> {
    let res = r
        .clone()
        .oneshot(
            Request::get(uri)
                .header("host", gitterrace_testkit::on_port("localhost"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("res");
    res.headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}
