//! Class guards from the threat model (`docs/security.md`). Each test walks the routes of the
//! built routers themselves, so a route registered anywhere is covered without a new test.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::app::Env;
use crate::public::{LOCAL, PUBLIC};

/// Text that must never reach a response: it lives outside the repository.
const SECRET: &str = "outside-the-repo-7f3a";

/// Every method a route could accept. A 405 answer means the route does not take it.
const METHODS: [&str; 6] = ["GET", "HEAD", "POST", "PUT", "DELETE", "PATCH"];

/// Query names that may carry a path. Every route gets them, so a new `?path=` is driven too.
const PATH_PARAMS: [&str; 3] = ["path", "file", "doc"];

/// Where a route is registered: the server itself, or each repository under `/r/{name}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Server,
    Repo,
}

struct Built {
    router: Router,
    secret: PathBuf,
    add_token: String,
    dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// A repository named `r` with a committed symlink `link`, an untracked symlink `out`, both
/// aiming at a secret file outside it, and a committed folder `d` swapped for a link to the
/// secret's folder.
fn hostile(root: &Path) -> (PathBuf, PathBuf) {
    std::fs::create_dir_all(root).expect("root");
    let secret = root.join("secret.txt");
    std::fs::write(&secret, format!("{SECRET}\n")).expect("secret");
    let p = root.join("r");
    let f = gitterrace_testkit::Fixture::init(&p)
        .write("a.txt", "a\n")
        .write("d/secret.txt", "inside\n");
    std::os::unix::fs::symlink(&secret, p.join("link")).expect("link");
    let repo = f.commit("feat: one").path().to_path_buf();
    std::os::unix::fs::symlink(&secret, repo.join("out")).expect("out");
    // The committed folder `d` becomes a link to the folder holding the secret, so its
    // tracked file now resolves outside the tree.
    std::fs::remove_dir_all(repo.join("d")).expect("rm d");
    std::os::unix::fs::symlink(root, repo.join("d")).expect("d");
    (repo, secret)
}

fn env(public: bool, settings: Option<&Path>) -> Env {
    let on = gitterrace_config::Feature::ALL
        .map(|f| [f.key(), " = true\n"].concat())
        .concat();
    let text = format!("[server]\nis_public = {public}\n[features]\n{on}");
    let mut env = Env::new(gitterrace_config::parse(&text).expect("parse").config);
    env.settings_dir = settings.map(Path::to_path_buf);
    env
}

/// A server over the hostile repository with actions, settings, repository admin and every
/// feature on, so every route that can be mounted is.
fn server(root: &Path, public: bool) -> Built {
    let (repo, secret) = hostile(root);
    let actions = crate::actions::actions_tests::config_dir(crate::actions::actions_tests::ECHO);
    let settings = tempfile::tempdir().expect("tmp");
    std::fs::write(settings.path().join("config.toml"), "").expect("w");
    let env = env(public, Some(settings.path()));
    let add = gitterrace_actions::Token::generate().expect("token");
    let add_token = add.as_str().to_owned();
    let file = actions.path().join("actions.toml");
    let router = crate::build_server(vec![repo], None, Some(&file), &env, Some(add), None)
        .expect("router")
        .0;
    Built {
        router,
        secret,
        add_token,
        dirs: (actions, settings),
    }
}

/// The route patterns a built router holds, read from its `Debug` form: the router is the
/// registry, so no list of source files can miss one.
fn patterns(r: &Router) -> BTreeSet<String> {
    let re = regex::Regex::new(r#"RouteId\(\d+\): "((?:[^"\\]|\\.)*)""#).expect("re");
    re.captures_iter(&format!("{r:?}"))
        .map(|c| c[1].to_owned())
        .filter(|p| !p.contains("__private__"))
        .collect()
}

/// Every route of the server and of one repository's router, with its scope. Call it inside
/// a runtime: the repository router starts its index refresher.
fn every_route(b: &Built) -> BTreeSet<(Scope, String)> {
    let (repo, _) = hostile(&b.secret.parent().expect("root").join("probe"));
    let one = crate::repo_router(&repo, "/r/r".into(), "r".into(), None, &env(false, None))
        .expect("repo router");
    let mut out: BTreeSet<_> = patterns(&b.router)
        .into_iter()
        .map(|p| (Scope::Server, p))
        .collect();
    out.extend(patterns(&one).into_iter().map(|p| (Scope::Repo, p)));
    assert!(out.len() > 50, "only {} routes found: {out:?}", out.len());
    out
}

/// The concrete URI of a route pattern on the one-repository server, with `tail` for a tail.
fn uri(scope: Scope, route: &str, tail: &str) -> String {
    let filled = route
        .replace("{rev}", "HEAD")
        .replace("{id}", "HEAD")
        .replace("{index}", "0")
        .replace("{name}", "r")
        .replace("{*path}", tail)
        .replace("{*rest}", tail);
    match scope {
        Scope::Server => filled,
        Scope::Repo => format!("/r/r{filled}"),
    }
}

struct Reply {
    status: u16,
    headers: axum::http::HeaderMap,
    body: String,
}

/// A page served here: same-origin fetch metadata, this origin, and `token` when given.
fn page(token: Option<&str>) -> Vec<(&'static str, String)> {
    let host = gitterrace_testkit::on_port("localhost");
    let mut h = vec![
        ("sec-fetch-site", "same-origin".to_owned()),
        ("origin", format!("http://{host}")),
    ];
    h.extend(token.map(|t| ("x-gitterrace-token", t.to_owned())));
    h
}

/// Sends a JSON `{}` request. A streaming body (`/events`) is read for a moment only.
async fn send(r: &Router, method: &str, uri: &str, headers: &[(&str, String)]) -> Reply {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", gitterrace_testkit::on_port("localhost"))
        .header("content-type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, v);
    }
    let res = r
        .clone()
        .oneshot(req.body(Body::from("{}")).expect("req"))
        .await
        .expect("res");
    let (status, headers) = (res.status().as_u16(), res.headers().clone());
    let body = tokio::time::timeout(Duration::from_millis(500), res.into_body().collect())
        .await
        .ok()
        .and_then(Result::ok)
        .map(|b| String::from_utf8_lossy(&b.to_bytes()).into_owned())
        .unwrap_or_default();
    Reply {
        status,
        headers,
        body,
    }
}

/// The `content` of `<meta name="{name}">` in a page.
fn meta(html: &str, name: &str) -> String {
    let key = format!("name=\"{name}\" content=\"");
    let rest = &html[html.find(&key).unwrap_or_else(|| panic!("{name} meta")) + key.len()..];
    rest[..rest.find('"').expect("end")].to_owned()
}

/// Class: every route of the built routers is either public (`public_route`) or listed in
/// `LOCAL`, never both, and neither table names a route that no router holds.
#[tokio::test(flavor = "multi_thread")]
async fn every_routed_pattern_is_classified_once() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    let routed: BTreeSet<String> = every_route(&b).into_iter().map(|(_, p)| p).collect();
    for r in &routed {
        let (public, local) = (crate::public::public_route(r), LOCAL.contains(&r.as_str()));
        assert!(
            public != local,
            "{r}: classify it in public.rs (public {public}, local {local})"
        );
    }
    for r in PUBLIC.iter().chain(LOCAL) {
        assert!(
            routed.contains(*r),
            "{r} is classified but no router holds it"
        );
    }
    // The sitemap documents every page route; assets, the action plumbing and the API (its
    // own registry) aside.
    let pages: BTreeSet<String> = routed
        .into_iter()
        .filter(|r| {
            !["/static", "/_actions", "/api/v1"]
                .iter()
                .any(|p| r.starts_with(p))
        })
        .collect();
    let documented: BTreeSet<String> = crate::sitemap::PAGES
        .iter()
        .flat_map(|p| p.routes.iter().map(|r| (*r).to_owned()))
        .collect();
    assert_eq!(
        pages, documented,
        "add or remove the route's entry in sitemap.rs"
    );
}

/// Class: a public server answers 404 on every route that is not public, under every method,
/// and each of its two walls holds on its own: `only_public` refuses the route, and so do the
/// feature switches.
#[tokio::test(flavor = "multi_thread")]
async fn every_local_route_is_dark_on_a_public_server() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), true);
    let cfg = env(true, None).cfg();
    let mut dark = 0;
    for (scope, route) in every_route(&server(&tmp.path().join("l"), false)) {
        if !LOCAL.contains(&route.as_str()) {
            continue;
        }
        assert!(
            !crate::public::public_route(&route),
            "{route}: only_public lets it through"
        );
        assert!(
            !crate::features::route_on(&cfg, None, &route),
            "{route}: no local-only switch turns it off when public"
        );
        let u = uri(scope, &route, "a.txt");
        for m in METHODS {
            let got = send(&b.router, m, &u, &[]).await.status;
            let ok = got == 404 || (m != "GET" && m != "HEAD" && got == 405);
            assert!(ok, "{m} {u} answered {got} on a public server");
        }
        dark += 1;
    }
    assert!(dark >= LOCAL.len(), "only {dark} local routes driven");
}

/// Class: every response, under every method a route takes and with the token a page holds,
/// carries nosniff and the same-origin resource policy, and every HTML response a CSP that
/// refuses foreign and inline script, so repository content rendered into any page cannot run.
#[tokio::test(flavor = "multi_thread")]
async fn every_response_sends_nosniff_and_html_sends_the_csp() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    let actions = meta(
        &send(&b.router, "GET", "/r/r/", &[]).await.body,
        "gitterrace-actions",
    );
    let settings = meta(
        &send(&b.router, "GET", "/_settings", &[]).await.body,
        "gitterrace-settings",
    );
    let callers = [
        page(Some(&actions)),
        page(Some(&settings)),
        vec![(crate::add::TOKEN_HEADER, b.add_token.clone())],
    ];
    let mut accepted = 0;
    for (scope, route) in every_route(&b) {
        let u = uri(scope, &route, "a.txt");
        for m in METHODS {
            for who in &callers {
                let res = send(&b.router, m, &u, who).await;
                accepted += usize::from(res.status != 405);
                assert_safe_headers(m, &u, &res);
            }
        }
    }
    assert!(accepted > 150, "only {accepted} accepted requests");
}

fn assert_safe_headers(m: &str, u: &str, res: &Reply) {
    let h = &res.headers;
    let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or_default();
    assert_eq!(get("x-content-type-options"), "nosniff", "{m} {u}");
    assert_eq!(
        get("cross-origin-resource-policy"),
        "same-origin",
        "{m} {u}"
    );
    if get("content-type").starts_with("text/html") {
        let csp = get("content-security-policy");
        assert!(
            csp.contains("script-src 'self'") || csp.contains("sandbox"),
            "{m} {u}: {csp}"
        );
        assert!(!csp.contains("'unsafe-eval'"), "{m} {u}: {csp}");
    }
}

/// Class: no path a request names reaches outside the tree. Every path tail and every
/// path-like query parameter on every route is driven with `.git`, `..` (plain and encoded),
/// an absolute path and a symlink out (committed and untracked): no response shows the
/// secret beside the repository or the repository's own config.
#[tokio::test(flavor = "multi_thread")]
async fn no_path_route_reads_outside_the_tree() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    let abs = b.secret.to_str().expect("utf8").to_owned();
    let tails = [
        ".git/config",
        "../secret.txt",
        "%2e%2e/secret.txt",
        "a/%2e%2e/%2e%2e/secret.txt",
        "..%2fsecret.txt",
        "out",
        "out/",
        "link",
        "d/secret.txt",
        abs.trim_start_matches('/'),
    ];
    let values = [
        ".git/config",
        "../secret.txt",
        "out",
        "link",
        "d/secret.txt",
        "/.git/config",
        &abs,
    ];
    let mut checked = 0;
    for (scope, route) in every_route(&b) {
        let mut uris: Vec<String> = Vec::new();
        if route.contains("{*") {
            uris.extend(tails.iter().map(|t| uri(scope, &route, t)));
        }
        let plain = uri(scope, &route, "a.txt");
        uris.push(plain.clone());
        for v in values {
            let q: Vec<String> = PATH_PARAMS.iter().map(|n| format!("{n}={v}")).collect();
            uris.push(format!("{plain}?{}", q.join("&")));
        }
        for u in uris {
            let res = send(&b.router, "GET", &u, &page(None)).await;
            assert!(!res.body.contains(SECRET), "{u}: {}", res.body);
            assert!(!res.body.contains("[core]"), "{u}: {}", res.body);
            checked += 1;
        }
    }
    assert!(checked > 400, "only {checked} path requests");
}

/// Class: no route changes state for a same-origin request without its token. Every method
/// but GET and HEAD on every route answers 403, 404, 405 or 415, never success.
#[tokio::test(flavor = "multi_thread")]
async fn no_write_succeeds_without_the_token() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    for (scope, route) in every_route(&b) {
        let u = uri(scope, &route, "a.txt");
        for m in ["POST", "PUT", "DELETE", "PATCH"] {
            let got = send(&b.router, m, &u, &page(None)).await.status;
            assert!(
                matches!(got, 403 | 404 | 405 | 415),
                "{m} {u} answered {got} without a token"
            );
        }
    }
    let after = std::fs::read_to_string(b.dirs.1.path().join("config.toml")).expect("r");
    assert_eq!(after, "", "a tokenless request wrote the config");
}
