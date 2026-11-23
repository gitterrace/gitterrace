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

/// A repository named `r` with a committed file and folder, and a secret file outside it. On
/// unix the repository also holds a committed symlink `link`, an untracked symlink `out`, both
/// aiming at the secret, and the committed folder `d` swapped for a link to the secret's
/// folder. Windows needs a privilege to make links, so there the path classes run without them.
fn hostile(root: &Path) -> (PathBuf, PathBuf) {
    std::fs::create_dir_all(root).expect("root");
    let secret = root.join("secret.txt");
    std::fs::write(&secret, format!("{SECRET}\n")).expect("secret");
    let p = root.join("r");
    let f = gitterrace_testkit::Fixture::init(&p)
        .write("a.txt", "a\n")
        .write("d/secret.txt", "inside\n");
    links::commit_link(&secret, &p);
    let repo = f.commit("feat: one").path().to_path_buf();
    links::swap_in_links(root, &secret, &repo);
    (repo, secret)
}

#[cfg(unix)]
mod links {
    use std::path::Path;

    pub(super) fn commit_link(secret: &Path, repo: &Path) {
        std::os::unix::fs::symlink(secret, repo.join("link")).expect("link");
    }

    pub(super) fn swap_in_links(root: &Path, secret: &Path, repo: &Path) {
        std::os::unix::fs::symlink(secret, repo.join("out")).expect("out");
        // The committed folder `d` becomes a link to the folder holding the secret, so its
        // tracked file now resolves outside the tree.
        std::fs::remove_dir_all(repo.join("d")).expect("rm d");
        std::os::unix::fs::symlink(root, repo.join("d")).expect("d");
    }
}

#[cfg(not(unix))]
mod links {
    use std::path::Path;

    pub(super) fn commit_link(_secret: &Path, _repo: &Path) {}

    pub(super) fn swap_in_links(_root: &Path, _secret: &Path, _repo: &Path) {}
}

/// A folder holding an `actions.toml` with one action, so the action routes are mounted. Unix
/// uses the shared fixture, whose loader demands private modes. Elsewhere a plain file does.
#[cfg(unix)]
fn actions_dir() -> tempfile::TempDir {
    crate::actions::actions_tests::config_dir(crate::actions::actions_tests::ECHO)
}

#[cfg(not(unix))]
fn actions_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tmp");
    let body = "version = 1\n[[action]]\nid = \"echo\"\nlabel = \"Echo\"\n\
        kinds = [\"line\", \"commit\", \"page\", \"file\"]\nshell = true\n\
        command = \"echo hi\"\ntimeout = \"5s\"\n";
    std::fs::write(d.path().join("actions.toml"), body).expect("write");
    d
}

fn env(public: bool, settings: Option<&Path>) -> Env {
    let on = gitterrace_config::Feature::ALL.map(|f| [f.key(), " = true\n"].concat()).concat();
    let text = format!("[server]\nis_public = {public}\n[features]\n{on}");
    let mut env = Env::new(gitterrace_config::parse(&text).expect("parse").config);
    env.settings_dir = settings.map(Path::to_path_buf);
    env
}

/// A server over the hostile repository with actions, settings, repository admin and every
/// feature on, so every route that can be mounted is.
fn server(root: &Path, public: bool) -> Built {
    let (repo, secret) = hostile(root);
    let actions = actions_dir();
    let settings = tempfile::tempdir().expect("tmp");
    std::fs::write(settings.path().join("config.toml"), "").expect("w");
    let env = env(public, Some(settings.path()));
    let add = gitterrace_actions::Token::generate().expect("token");
    let add_token = add.as_str().to_owned();
    let file = actions.path().join("actions.toml");
    let router = crate::build_server(vec![repo], None, Some(&file), &env, Some(add), None)
        .expect("router")
        .0;
    Built { router, secret, add_token, dirs: (actions, settings) }
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
        .expect("repo router")
        .0;
    let mut out: BTreeSet<_> =
        patterns(&b.router).into_iter().map(|p| (Scope::Server, p)).collect();
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

/// What a page served here sends, or what any program can forge: same-origin fetch metadata and
/// this origin, plus the `cookie` pair when given.
fn page(cookie: Option<&str>) -> Vec<(&'static str, String)> {
    let host = gitterrace_testkit::on_port("localhost");
    let mut h =
        vec![("sec-fetch-site", "same-origin".to_owned()), ("origin", format!("http://{host}"))];
    h.extend(cookie.map(|t| ("cookie", t.to_owned())));
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
    let res = r.clone().oneshot(req.body(Body::from("{}")).expect("req")).await.expect("res");
    let (status, headers) = (res.status().as_u16(), res.headers().clone());
    let body = tokio::time::timeout(Duration::from_millis(500), res.into_body().collect())
        .await
        .ok()
        .and_then(Result::ok)
        .map(|b| String::from_utf8_lossy(&b.to_bytes()).into_owned())
        .unwrap_or_default();
    Reply { status, headers, body }
}

/// Every `content` of the `<meta>` tags that once carried a write token, plus the long
/// base64url words in any `content` or `value`: what a page could hand a forging client.
fn token_like(html: &str) -> Vec<String> {
    let re =
        regex::Regex::new(r#"(?:content|value|data-[a-z-]+)="([A-Za-z0-9_-]{32,})""#).expect("re");
    // A commit or repository id is hex and public: a token is base64url and has other letters.
    re.captures_iter(html)
        .map(|c| c[1].to_owned())
        .filter(|w| !w.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect()
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
        assert!(public != local, "{r}: classify it in public.rs (public {public}, local {local})");
    }
    for r in PUBLIC.iter().chain(LOCAL) {
        assert!(routed.contains(*r), "{r} is classified but no router holds it");
    }
    // The sitemap documents every page route; assets, the action plumbing and the API (its
    // own registry) aside.
    let pages: BTreeSet<String> = routed
        .into_iter()
        .filter(|r| !["/static", "/_actions", "/api/v1"].iter().any(|p| r.starts_with(p)))
        .collect();
    let documented: BTreeSet<String> = crate::sitemap::PAGES
        .iter()
        .flat_map(|p| p.routes.iter().map(|r| (*r).to_owned()))
        .collect();
    assert_eq!(pages, documented, "add or remove the route's entry in sitemap.rs");
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
        assert!(!crate::public::public_route(&route), "{route}: only_public lets it through");
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
/// Every HTML response also says how long it may be cached, so no browser guesses one for a
/// page named by a moving ref.
#[tokio::test(flavor = "multi_thread")]
async fn every_response_sends_nosniff_and_html_sends_the_csp() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    let callers = [
        page(Some(&crate::owner::test_cookie())),
        page(None),
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
    assert_eq!(get("cross-origin-resource-policy"), "same-origin", "{m} {u}");
    if get("content-type").starts_with("text/html") {
        let csp = get("content-security-policy");
        assert!(csp.contains("script-src 'self'") || csp.contains("sandbox"), "{m} {u}: {csp}");
        assert!(!csp.contains("'unsafe-eval'"), "{m} {u}: {csp}");
        assert!(!get("cache-control").is_empty(), "{m} {u}: no cache-control");
    }
}

/// A page at HEAD revalidates every time, a hashed asset keeps its year and raw its validator.
#[tokio::test(flavor = "multi_thread")]
async fn moving_pages_revalidate_and_assets_keep_their_lifetime() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    let cache = |r: &Reply| {
        let v = r.headers.get("cache-control").expect("cache-control");
        v.to_str().expect("ascii").to_owned()
    };
    let blob = send(&b.router, "GET", "/r/r/blob/HEAD/a.txt", &[]).await;
    assert_eq!((blob.status, cache(&blob).as_str()), (200, "private, no-cache"));
    let js = send(&b.router, "GET", crate::MERMAID_JS, &[]).await;
    assert_eq!(cache(&js), "public, max-age=31536000, immutable");
    let raw = send(&b.router, "GET", "/r/r/raw/HEAD/a.txt", &[]).await;
    assert_eq!(cache(&raw), "private, max-age=300");
    assert!(raw.headers.contains_key("etag"));
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
    let values =
        [".git/config", "../secret.txt", "out", "link", "d/secret.txt", "/.git/config", &abs];
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

/// The routes that take a write, as (pattern, method): a write is an owner decision recorded
/// here, never an accident. Each is mounted only when its feature is configured.
const WRITES: [(&str, &str); 9] = [
    (crate::actions::RUN_PATH, "POST"),
    ("/_launch", "POST"),
    ("/_repos", "POST"),
    ("/_repos", "DELETE"),
    ("/_settings/file", "PUT"),
    ("/_settings/check", "POST"),
    ("/_checkpoint", "POST"),
    ("/_viewed", "POST"),
    ("/_viewed/all", "POST"),
];

/// Class: a plain client on a shared loopback, with every header a non-browser can forge and no
/// owner proof, changes nothing anywhere. The write routes are found from the routers: any
/// route that does not answer 405 to a non-GET method is one, so a new write route is driven
/// without a new test, and must be listed in `WRITES` (an owner decision). Every other
/// method answers 405 on every route. The forger also reads every page first and replays
/// whatever token-looking value it finds, since that is the attack: it must not help.
#[tokio::test(flavor = "multi_thread")]
async fn a_forging_loopback_client_without_owner_proof_writes_nothing() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    let mut loot = BTreeSet::new();
    for u in ["/", "/r/r/", "/r/r/log/HEAD", "/_settings", "/_settings?f=actions"] {
        loot.extend(token_like(&send(&b.router, "GET", u, &[]).await.body));
    }
    let mut forged: Vec<Vec<(&str, String)>> = vec![page(None)];
    for t in &loot {
        forged.push(page(Some(t)));
        forged.push(page(Some(&format!("gt-owner-x={t}"))));
        let mut h = page(None);
        h.push(("x-gitterrace-token", t.clone()));
        h.push((crate::add::TOKEN_HEADER, t.clone()));
        forged.push(h);
    }
    let mut found = BTreeSet::new();
    for (scope, route) in every_route(&b) {
        let u = uri(scope, &route, "a.txt");
        let missing = send(&b.router, "GET", &u, &page(None)).await.status == 404;
        for m in ["POST", "PUT", "DELETE", "PATCH"] {
            for h in &forged {
                let got = send(&b.router, m, &u, h).await.status;
                if got == 403 {
                    found.insert((route.clone(), m));
                }
                // 405 is a route that exists for GET only. A URL GET cannot find either may
                // answer 404. Any other status means a handler took the write.
                assert!(
                    got == 403 || got == 405 || (missing && got == 404),
                    "{m} {u} answered {got} to a forged client"
                );
            }
        }
    }
    let want: BTreeSet<_> = WRITES.iter().map(|(r, m)| ((*r).to_owned(), *m)).collect();
    assert_eq!(found, want, "the write routes the routers hold differ from WRITES");
    for h in &forged {
        let spent = send(&b.router, "GET", "/_launch?t=guess&to=/", h).await;
        assert_eq!(spent.status, 403, "a guessed ticket");
        assert!(!spent.headers.contains_key("set-cookie"), "a cookie for a guess");
    }
    let after = std::fs::read_to_string(b.dirs.1.path().join("config.toml")).expect("r");
    assert_eq!(after, "", "a forged request wrote the config");
}

/// Class: the owner cookie opens every write route past the guard. Without this the test above
/// would pass on routes that refuse everyone.
#[tokio::test(flavor = "multi_thread")]
async fn the_owner_cookie_reaches_every_write_route() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    let who = page(Some(&crate::owner::test_cookie()));
    for (route, m) in WRITES {
        if route == "/_launch" || route == "/_repos" {
            continue;
        }
        // The page writes belong to a repository, so they sit under its prefix.
        let at = if route.starts_with("/_checkpoint") || route.starts_with("/_viewed") {
            format!("/r/r{route}")
        } else {
            route.to_owned()
        };
        let got = send(&b.router, m, &at, &who).await.status;
        assert!(got != 403 && got != 405 && got != 404, "{m} {at} answered {got} to the owner");
    }
}

/// Class: no served page holds a write token or any secret. Another user of the machine reads
/// every page over loopback, so a page that embeds one is an oracle for it. Every page of the
/// built routers is read, with and without the owner cookie, and the owner proof, the add
/// secret and a ticket must appear in none of them, nor in a header.
#[tokio::test(flavor = "multi_thread")]
async fn no_served_page_contains_a_write_token() {
    let tmp = tempfile::tempdir().expect("tmp");
    let b = server(tmp.path(), false);
    let cookie = crate::owner::test_cookie();
    let proof = cookie.split_once('=').expect("pair").1.to_owned();
    let mut seen = 0;
    for (scope, route) in every_route(&b) {
        let u = uri(scope, &route, "a.txt");
        for who in [page(None), page(Some(&cookie))] {
            let res = send(&b.router, "GET", &u, &who).await;
            let all = format!("{:?}{}", res.headers, res.body);
            for secret in [&proof, &b.add_token] {
                assert!(!all.contains(secret.as_str()), "{u} holds a secret");
            }
            assert!(token_like(&res.body).is_empty(), "{u} holds a token-like value");
            assert!(!res.body.to_lowercase().contains("x-gitterrace-token"), "{u}");
            seen += 1;
        }
    }
    assert!(seen > 100, "only {seen} pages read");
}
