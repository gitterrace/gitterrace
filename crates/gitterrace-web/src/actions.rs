//! Actions and permalinks over HTTP. `/go/<repo-id>/<sha>[/<path>]` finds the served repo a
//! permalink names. `POST /_actions/run` is gitterrace's only state-changing route: it exists
//! only when the user's `actions.toml` declares at least one action, `--no-actions` is
//! absent and the server is bound to loopback. Its checks are stricter than the GET guard:
//! same-origin fetch metadata, a matching `Origin`, the per-process token and a JSON body.
//! The request names an action id and a locator, never a command; `gitterrace-actions`
//! builds the payload from git and runs the user's argv.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

use axum::{
    Router,
    extract::{Path as UrlPath, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use gitterrace_actions::{
    Action, Config, Limits, Output, Payload, PayloadError, Reloader, RunError, RunRequest, Runner,
    Token,
};
use gitterrace_git::Repo;

use crate::fmt::{enc, esc};

pub(crate) const RUN_PATH: &str = "/_actions/run";
const STATUS_PATH: &str = "/_actions/status";
/// The request body is an id, a kind and a locator; anything near this is not a click.
const BODY_LIMIT: usize = 8 * 1024;
/// How often `actions.toml` is checked for edits.
const RELOAD_POLL: Duration = Duration::from_secs(1);

/// One served repository as permalinks and run requests see it.
pub(crate) struct Target {
    /// The name a locator uses: the URL name with several repos, the folder name with one.
    pub(crate) key: String,
    /// URL prefix of its pages: empty, or `/r/NAME`.
    pub(crate) base: String,
    pub(crate) repo: Repo,
}

impl AsRef<Repo> for Target {
    fn as_ref(&self) -> &Repo {
        &self.repo
    }
}

/// Actions switched on: the token pages embed and the live action set.
pub(crate) struct Live {
    token: Token,
    config: Arc<RwLock<Arc<Config>>>,
    runner: Runner,
}

impl Live {
    pub(crate) fn token(&self) -> &str {
        self.token.as_str()
    }

    fn config(&self) -> Arc<Config> {
        self.config
            .read()
            .map_or_else(|_| Arc::default(), |c| Arc::clone(&c))
    }

    /// The menu entries the client renders: id, label, kinds and shortcut, in file order.
    pub(crate) fn menu_json(&self) -> String {
        let items: Vec<_> = self
            .config()
            .actions
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "label": a.label,
                    "kinds": a.kinds,
                    "key": a.key.map(String::from),
                })
            })
            .collect();
        serde_json::Value::from(items).to_string()
    }
}

/// Why `actions.toml` is not in force, for the note pages show. Empty when it is fine.
#[derive(Default)]
pub(crate) struct Status(RwLock<Option<String>>);

impl Status {
    fn set(&self, note: Option<String>) {
        if let Ok(mut g) = self.0.write() {
            *g = note;
        }
    }

    /// The line to show, `actions.toml not loaded: <error>`, or `None`.
    pub(crate) fn note(&self) -> Option<String> {
        self.0.read().ok().and_then(|g| g.clone())
    }
}

fn not_loaded(e: impl std::fmt::Display) -> String {
    format!("actions.toml not loaded: {e}")
}

/// Everything `/go` and the run route read.
pub(crate) struct Site {
    pub(crate) targets: Vec<Target>,
    pub(crate) live: Option<Arc<Live>>,
    pub(crate) status: Arc<Status>,
    /// The config `[[send]]` targets come from: the live, reloaded one when actions are on.
    pub(crate) sends: Arc<RwLock<Arc<Config>>>,
    /// The server's settings, read per request so a reload of `links.style` reaches payloads.
    pub(crate) env: crate::app::Env,
}

impl Site {
    fn link_style(&self) -> gitterrace_actions::LinkStyle {
        gitterrace_actions::LinkStyle::from_config(&self.env.cfg().links.style)
    }
}

/// Turns actions on when the file at `file` is safe and declares at least one action, and
/// the server is bound to loopback. `None` (the route is never mounted) otherwise; a later
/// edit cannot turn it on, only a restart. A file that is refused is reported through the
/// returned [`Status`] and the log. Starts the reload poll, so call inside tokio.
pub(crate) fn start(
    file: Option<&Path>,
    roots: &[PathBuf],
    loopback: bool,
) -> (Option<Arc<Live>>, Arc<Status>) {
    let status = Arc::new(Status::default());
    (start_live(file, roots, loopback, &status), status)
}

/// Where `GET /_send/targets` reads from. Sending opens a URL in the browser and starts
/// nothing, so it needs neither loopback nor an action; a refused file sends nowhere.
pub(crate) fn sends(
    live: Option<&Arc<Live>>,
    file: Option<&Path>,
    roots: &[PathBuf],
) -> Arc<RwLock<Arc<Config>>> {
    if let Some(l) = live {
        return Arc::clone(&l.config);
    }
    let config = file
        .filter(|f| std::fs::symlink_metadata(f).is_ok())
        .and_then(|f| Config::load(f, roots).ok())
        .unwrap_or_default();
    Arc::new(RwLock::new(Arc::new(config)))
}

fn start_live(
    file: Option<&Path>,
    roots: &[PathBuf],
    loopback: bool,
    status: &Arc<Status>,
) -> Option<Arc<Live>> {
    let file = file?;
    if std::fs::symlink_metadata(file).is_err() {
        return None;
    }
    if !loopback {
        tracing::warn!("actions disabled: not bound to loopback");
        return None;
    }
    let config = match Config::load(file, roots) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("actions disabled: {e}");
            status.set(Some(not_loaded(&e)));
            tokio::spawn(retry(
                file.to_path_buf(),
                roots.to_vec(),
                Arc::clone(status),
            ));
            return None;
        }
    };
    if config.actions.is_empty() {
        return None;
    }
    let token = match Token::generate() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("actions disabled: no randomness for the token: {e}");
            return None;
        }
    };
    tracing::info!(
        "actions: {} loaded from {}",
        config.actions.len(),
        file.display()
    );
    let reloader = Reloader::new(file.to_path_buf(), roots.to_vec(), config);
    let live = Arc::new(Live {
        token,
        config: reloader.handle(),
        runner: Runner::new(Limits::default()),
    });
    tokio::spawn(reload(reloader, Arc::clone(status)));
    Some(live)
}

async fn reload(mut reloader: Reloader, status: Arc<Status>) {
    loop {
        tokio::time::sleep(RELOAD_POLL).await;
        let Ok((back, lines)) = tokio::task::spawn_blocking(move || {
            let lines = reloader.poll();
            (reloader, lines)
        })
        .await
        else {
            return;
        };
        reloader = back;
        status.set(reloader.error().map(not_loaded));
        for l in lines {
            tracing::info!("{l}");
        }
    }
}

/// After a refused first load: keeps the note current as the file is edited. A valid file
/// cannot turn actions on without a restart, so the note says so instead of vanishing.
async fn retry(file: PathBuf, roots: Vec<PathBuf>, status: Arc<Status>) {
    let mut reloader = Reloader::new(file, roots, Config::default());
    let handle = reloader.handle();
    loop {
        tokio::time::sleep(RELOAD_POLL).await;
        let Ok((back, lines)) = tokio::task::spawn_blocking(move || {
            let lines = reloader.poll();
            (reloader, lines)
        })
        .await
        else {
            return;
        };
        reloader = back;
        // An unchanged file keeps the note startup set.
        if lines.is_empty() {
            continue;
        }
        let usable = handle.read().is_ok_and(|c| !c.actions.is_empty());
        let note = match reloader.error() {
            Some(e) => Some(not_loaded(e)),
            None if usable => {
                Some("actions.toml is valid now: restart gitterrace to enable actions".to_owned())
            }
            None => None,
        };
        status.set(note);
        for l in lines {
            tracing::info!("{l}");
        }
    }
}

/// `/go/...` always; the run route only when actions are on.
pub(crate) fn router(site: Arc<Site>) -> Router {
    let r = Router::new()
        .route("/go/{*rest}", get(go))
        .route(STATUS_PATH, get(status))
        .route("/_send/targets", get(send_targets))
        .route("/r/{name}/_send/targets", get(send_targets));
    let r = if site.live.is_some() {
        r.route(RUN_PATH, post(run))
    } else {
        r
    };
    r.with_state(site)
}

/// `GET /_actions/status`: `{"note": string|null}`, fetched by `actions.js` on load, focus and tab show.
async fn status(State(site): State<Arc<Site>>) -> Response {
    let body = serde_json::json!({ "note": site.status.note() });
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// `GET /_send/targets`: `[{"id","label","parts":[{"lit"}|{"ph"}]}]`, `[]` with none.
/// The same list under every repository prefix, since targets are not per repository.
async fn send_targets(State(site): State<Arc<Site>>) -> Response {
    let config = site
        .sends
        .read()
        .map_or_else(|_| Arc::default(), |c| Arc::clone(&c));
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::to_string(&config.sends).unwrap_or_else(|_| "[]".to_owned()),
    )
        .into_response()
}

async fn go(State(site): State<Arc<Site>>, UrlPath(rest): UrlPath<String>) -> Response {
    let mut parts = rest.splitn(3, '/');
    let id = parts.next().unwrap_or_default().to_owned();
    let sha = parts.next().unwrap_or_default().to_owned();
    let path = parts
        .next()
        .map(|p| p.trim_matches('/').to_owned())
        .filter(|p| !p.is_empty());
    let found = tokio::task::spawn_blocking(move || {
        let t = gitterrace_git::resolve_permalink(&site.targets, &id, &sha, path.as_deref())?;
        let url = match &path {
            None => format!("{}/commit/{sha}", t.base),
            Some(p) if t.repo.blob(&sha, p).is_ok() => {
                format!("{}/blob/{sha}/{}", t.base, enc(p, true))
            }
            Some(p) => format!("{}/tree/{sha}/{}", t.base, enc(p, true)),
        };
        Some(url)
    })
    .await
    .ok()
    .flatten();
    if let Some(loc) = found.and_then(|u| HeaderValue::from_str(&u).ok()) {
        return (StatusCode::FOUND, [(header::LOCATION, loc)]).into_response();
    }
    let id = rest.split('/').next().unwrap_or_default();
    let body = format!(
        "<!doctype html><meta charset=utf-8><link rel=stylesheet href=/static/tokens.css>\
         <link rel=stylesheet href=/static/app.css><main class=wrap><h1>404</h1>\
         <p class=muted>Repository {} (or that commit) is not served here.</p>\
         <p><a href=/>Back to overview</a></p></main>",
        esc(id)
    );
    (StatusCode::NOT_FOUND, Html(body)).into_response()
}

/// Why a run request is refused before its body is read. Fetch metadata must say
/// same-origin (absent and `none` are refused, unlike GETs), `Origin` must be this host, and
/// the token must match: only a page served here can read it.
fn refusal(h: &HeaderMap, token: &Token) -> Option<&'static str> {
    let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok());
    if !get("sec-fetch-site").is_some_and(|v| v.eq_ignore_ascii_case("same-origin")) {
        return Some("forbidden: not a same-origin request\n");
    }
    let host = get("host").unwrap_or_default();
    let origin_ok = get("origin").is_some_and(|o| {
        o.strip_prefix("http://")
            .is_some_and(|rest| !host.is_empty() && rest.eq_ignore_ascii_case(host))
    });
    if !origin_ok {
        return Some("forbidden: foreign origin\n");
    }
    if !get("x-gitterrace-token").is_some_and(|t| token.verify(t)) {
        return Some("forbidden: bad token\n");
    }
    None
}

fn is_json(h: &HeaderMap) -> bool {
    h.get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
}

fn reply(status: StatusCode, ok: bool, exit: Option<i32>, first_line: &str) -> Response {
    let body = serde_json::json!({ "ok": ok, "exit": exit, "first_line": first_line });
    (
        status,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        axum::Json(body),
    )
        .into_response()
}

fn bad(why: &str) -> Response {
    reply(StatusCode::BAD_REQUEST, false, None, why)
}

enum Failed {
    Locator(String),
    Run(RunError),
    Internal,
}

async fn run(State(site): State<Arc<Site>>, req: Request) -> Response {
    let Some(live) = site.live.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(why) = refusal(req.headers(), &live.token) {
        return (StatusCode::FORBIDDEN, why).into_response();
    }
    if !is_json(req.headers()) {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "json only\n").into_response();
    }
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let Ok(bytes) = axum::body::to_bytes(req.into_body(), BODY_LIMIT).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "body too large\n").into_response();
    };
    let Ok(rr) = serde_json::from_slice::<RunRequest>(&bytes) else {
        return bad("malformed request");
    };
    let Some(action) = live.config().get(&rr.action).cloned() else {
        return bad("unknown action");
    };
    if !action.kinds.contains(&rr.kind) {
        return bad("this action does not take that kind");
    }
    let Some(idx) = site.targets.iter().position(|t| t.key == rr.locator.repo) else {
        return bad("unknown repository");
    };
    let origin = format!("http://{host}");
    let result =
        tokio::task::spawn_blocking(move || run_blocking(&site, idx, &live, &action, &rr, &origin))
            .await
            .unwrap_or(Err(Failed::Internal));
    match result {
        Ok((outcome, output)) => {
            let shown = if outcome.ok && output == Output::Ignore {
                ""
            } else {
                outcome.first_line.as_str()
            };
            reply(StatusCode::OK, outcome.ok, outcome.exit, shown)
        }
        Err(Failed::Locator(why)) => bad(&why),
        Err(Failed::Run(e @ (RunError::Busy | RunError::RateLimited))) => {
            reply(StatusCode::TOO_MANY_REQUESTS, false, None, &e.to_string())
        }
        Err(Failed::Run(RunError::Refused(why))) => {
            reply(StatusCode::UNPROCESSABLE_ENTITY, false, None, &why)
        }
        Err(Failed::Run(e)) => {
            tracing::warn!("action failed to start: {e}");
            reply(
                StatusCode::INTERNAL_SERVER_ERROR,
                false,
                None,
                "could not start the action, see the server log",
            )
        }
        Err(Failed::Internal) => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            false,
            None,
            "internal error",
        ),
    }
}

fn run_blocking(
    site: &Site,
    idx: usize,
    live: &Live,
    action: &Action,
    rr: &RunRequest,
    origin: &str,
) -> Result<(gitterrace_actions::Outcome, Output), Failed> {
    let t = site.targets.get(idx).ok_or(Failed::Internal)?;
    let style = site.link_style();
    let payload = Payload::build(&t.repo, &action.id, rr.kind, &rr.locator, origin, style)
        .map_err(|e| match e {
            PayloadError::BadLocator(why) => Failed::Locator(why),
            PayloadError::Git(e) => {
                tracing::warn!("action payload: {e}");
                Failed::Internal
            }
        })?;
    let outcome = live
        .runner
        .run(action, &payload, &t.repo.root())
        .map_err(Failed::Run)?;
    Ok((outcome, action.output))
}

/// Fixtures shared with the read-only crawl in `lib.rs`.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) mod actions_tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// A shell action that records its stdin and environment next to the config, then
    /// prints a first line. Placeholder `{dir}` is the config folder.
    pub(crate) const ECHO: &str = r#"version = 1
[[action]]
id = "echo"
label = "Echo"
kinds = ["line", "commit", "page", "file"]
shell = true
command = 'cat > "{dir}/stdin.json"; env > "{dir}/env.txt"; printf "%s\n" "$GITTERRACE_TITLE"'
timeout = "5s"

[[action]]
id = "slow"
label = "Slow"
kinds = ["page", "commit"]
shell = true
command = 'cat >/dev/null; sh -c "echo \$\$ > \"{dir}/grandchild.pid\"; sleep 30" & sleep 30'
timeout = "300ms"

[[action]]
id = "nap1"
label = "Nap 1"
kinds = ["page"]
shell = true
command = "cat >/dev/null; sleep 2"
[[action]]
id = "nap2"
label = "Nap 2"
kinds = ["page"]
shell = true
command = "cat >/dev/null; sleep 2"
[[action]]
id = "nap3"
label = "Nap 3"
kinds = ["page"]
shell = true
command = "cat >/dev/null; sleep 2"
[[action]]
id = "nap4"
label = "Nap 4"
kinds = ["page"]
shell = true
command = "cat >/dev/null; sleep 2"
"#;

    /// A private folder holding `actions.toml` built from `template`.
    pub(crate) fn config_dir(template: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tmp");
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
        write_config(d.path(), template, 0o600);
        d
    }

    pub(crate) fn write_config(dir: &Path, template: &str, mode: u32) {
        let p = dir.join("actions.toml");
        let body = template.replace("{dir}", &dir.to_string_lossy());
        std::fs::write(&p, body).expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::actions::RUN_PATH;
    use crate::actions::actions_tests::{ECHO, config_dir, write_config};

    /// Repo content aimed at the page and the shell: every field the menu touches.
    const XSS: &str = "<script>alert(1)</script>\"><img src=x onerror=alert(2)>";

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A repo whose subject, author, branch, path and content carry HTML and shell payloads,
    /// and which ships its own actions.toml (which must never load).
    fn hostile(root: &Path) -> (PathBuf, String) {
        let p = root.join("evil");
        std::fs::create_dir_all(p.join("src")).expect("dir");
        git(&p, &["init", "-q", "-b", "main"]);
        git(&p, &["config", "user.name", &format!("Eve {XSS}")]);
        git(&p, &["config", "user.email", "e@x.y"]);
        std::fs::write(
            p.join("src/a.rs"),
            format!("fn a() {{}}\n// {XSS}\n// $(touch pwned)\n"),
        )
        .expect("w");
        std::fs::write(p.join(format!("src/{}.txt", "q\"><img onerror=x>")), "x\n").expect("w");
        std::fs::write(p.join("actions.toml"), "version = 1\n").expect("w");
        std::fs::create_dir_all(p.join(".gitterrace")).expect("dir");
        std::fs::write(p.join(".gitterrace/actions.toml"), "version = 1\n").expect("w");
        git(&p, &["add", "."]);
        git(
            &p,
            &["commit", "-q", "-m", &format!("fix: $(touch pwned) {XSS}")],
        );
        git(&p, &["branch", "x\"><svg-onload=alert(3)>"]);
        let sha = git(&p, &["rev-parse", "HEAD"]);
        (p, sha)
    }

    fn router(repo: &Path, cfg: Option<&Path>, bound: Option<std::net::IpAddr>) -> axum::Router {
        crate::build_multi(
            vec![repo.to_path_buf()],
            bound,
            cfg.map(|d| d.join("actions.toml")).as_deref(),
            &crate::app::Env::new(gitterrace_config::Config::default()),
        )
        .expect("router")
        .0
    }

    async fn page(r: &axum::Router, uri: &str) -> (u16, String) {
        let res = r
            .clone()
            .oneshot(
                Request::get(uri)
                    .header("host", "localhost:4242")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        let s = res.status().as_u16();
        let b = res.into_body().collect().await.expect("body").to_bytes();
        (s, String::from_utf8_lossy(&b).into_owned())
    }

    fn token(html: &str) -> String {
        let i = html
            .find("name=\"gitterrace-actions\" content=\"")
            .expect("token meta");
        let rest = &html[i + 35..];
        rest[..rest.find('"').expect("end")].to_owned()
    }

    struct Req<'a> {
        token: Option<&'a str>,
        site: Option<&'a str>,
        host: &'a str,
        origin: Option<&'a str>,
        ctype: &'a str,
        body: String,
    }

    impl<'a> Req<'a> {
        fn ok(token: &'a str, body: String) -> Self {
            Self {
                token: Some(token),
                site: Some("same-origin"),
                host: "localhost:4242",
                origin: Some("http://localhost:4242"),
                ctype: "application/json",
                body,
            }
        }
    }

    async fn send(r: &axum::Router, q: Req<'_>) -> (u16, serde_json::Value) {
        let mut b = Request::post(RUN_PATH)
            .header("host", q.host)
            .header("content-type", q.ctype);
        if let Some(t) = q.token {
            b = b.header("x-gitterrace-token", t);
        }
        if let Some(s) = q.site {
            b = b.header("sec-fetch-site", s);
        }
        if let Some(o) = q.origin {
            b = b.header("origin", o);
        }
        let res = r
            .clone()
            .oneshot(b.body(Body::from(q.body)).expect("req"))
            .await
            .expect("res");
        let s = res.status().as_u16();
        let bytes = res.into_body().collect().await.expect("body").to_bytes();
        (
            s,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn body(action: &str, kind: &str, extra: &str) -> String {
        format!(
            r#"{{"action":"{action}","kind":"{kind}","locator":{{"repo":"evil","rev":"main"{extra}}}}}"#
        )
    }

    /// The design's table: every way a request can be wrong, then the one way it is right.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_route_security_table() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (repo, sha) = hostile(tmp.path());
        let cfg = config_dir(ECHO);

        // 1. not configured: no route, no token in pages.
        let none = tempfile::tempdir().expect("tmp");
        let off = router(&repo, Some(none.path()), None);
        let (_, html) = page(&off, "/").await;
        assert!(
            !html.contains("gitterrace-actions"),
            "token without actions"
        );
        let (s, _) = send(&off, Req::ok("x", body("echo", "page", ""))).await;
        assert_eq!(s, 404, "not configured");
        // 2. --no-actions: the CLI passes no file.
        let (s, _) = send(
            &router(&repo, None, None),
            Req::ok("x", body("echo", "page", "")),
        )
        .await;
        assert_eq!(s, 404, "--no-actions");
        // 3. bound to the LAN or to every interface.
        for ip in ["0.0.0.0", "192.168.1.5"] {
            let lan = router(&repo, Some(cfg.path()), Some(ip.parse().expect("ip")));
            let mut q = Req::ok("x", body("echo", "page", ""));
            q.host = if ip == "0.0.0.0" {
                "127.0.0.1:4242"
            } else {
                "192.168.1.5:4242"
            };
            q.origin = Some(if ip == "0.0.0.0" {
                "http://127.0.0.1:4242"
            } else {
                "http://192.168.1.5:4242"
            });
            assert_eq!(send(&lan, q).await.0, 404, "bound to {ip}");
        }

        let on = router(&repo, Some(cfg.path()), None);
        let (_, html) = page(&on, "/").await;
        let tok = token(&html);
        assert_eq!(tok.len(), 43);
        let wrong = {
            let mut b = tok.clone().into_bytes();
            b[5] = if b[5] == b'A' { b'B' } else { b'A' };
            String::from_utf8(b).expect("ascii")
        };
        let ok = || Req::ok(&tok, body("echo", "page", ""));
        // 4-5. missing and one-byte-off tokens.
        let mut q = ok();
        q.token = None;
        assert_eq!(send(&on, q).await.0, 403, "missing token");
        let mut q = ok();
        q.token = Some(&wrong);
        assert_eq!(send(&on, q).await.0, 403, "wrong token");
        // 6-8. fetch metadata: cross-site, same-site (another port), none and absent.
        for site in [Some("cross-site"), Some("same-site"), Some("none"), None] {
            let mut q = ok();
            q.site = site;
            assert_eq!(send(&on, q).await.0, 403, "sec-fetch-site {site:?}");
        }
        // 9. DNS rebinding.
        let mut q = ok();
        q.host = "evil.example";
        q.origin = Some("http://evil.example");
        assert_eq!(send(&on, q).await.0, 403, "foreign host");
        // 10. another localhost origin, and no origin at all.
        for origin in [Some("http://localhost:5173"), Some("null"), None] {
            let mut q = ok();
            q.origin = origin;
            assert_eq!(send(&on, q).await.0, 403, "origin {origin:?}");
        }
        // 11. not JSON.
        for ctype in ["text/plain", "application/x-www-form-urlencoded"] {
            let mut q = ok();
            q.ctype = ctype;
            assert_eq!(send(&on, q).await.0, 415, "{ctype}");
        }
        // 12. unknown action, kind not allowed, unknown field, a command field.
        for b in [
            body("nope", "page", ""),
            body("echo", "hunk", r#","path":"src/a.rs""#),
            body("echo", "page", r#","cwd":"/""#),
            r#"{"action":"echo","kind":"page","command":["sh"],"locator":{"repo":"evil","rev":"main"}}"#
                .to_owned(),
            body("echo", "page", "").replace("evil", "other"),
        ] {
            assert_eq!(send(&on, Req::ok(&tok, b.clone())).await.0, 400, "{b}");
        }
        // 13. path traversal.
        let (s, _) = send(
            &on,
            Req::ok(&tok, body("echo", "file", r#","path":"../../etc/passwd""#)),
        )
        .await;
        assert_eq!(s, 400, "traversal");

        // 14. valid: exact JSON on stdin, only allowed env. 15. $(touch pwned) not run.
        let (s, res) = send(&on, Req::ok(&tok, body("echo", "commit", ""))).await;
        assert_eq!(s, 200, "{res}");
        assert_eq!(res["ok"], true);
        assert!(
            res["first_line"]
                .as_str()
                .expect("line")
                .contains("$(touch pwned)")
        );
        assert!(!repo.join("pwned").exists() && !cfg.path().join("pwned").exists());
        let stdin: serde_json::Value =
            serde_json::from_slice(&std::fs::read(cfg.path().join("stdin.json")).expect("stdin"))
                .expect("json");
        assert_eq!(stdin["v"], 1);
        assert_eq!(stdin["kind"], "commit");
        assert_eq!(stdin["sha"], sha);
        assert_eq!(stdin["repo"]["name"], "evil");
        assert!(
            stdin["permalink"]
                .as_str()
                .expect("link")
                .starts_with("gitterrace://")
        );
        let env = std::fs::read_to_string(cfg.path().join("env.txt")).expect("env");
        for line in env.lines() {
            let name = line.split('=').next().unwrap_or_default();
            assert!(
                [
                    "PATH", "HOME", "USER", "LANG", "TMPDIR", "PWD", "SHLVL", "OLDPWD", "_"
                ]
                .contains(&name)
                    || name.starts_with("LC_")
                    || name.starts_with("GITTERRACE_"),
                "leaked {name}"
            );
        }
        let (s, res) = send(
            &on,
            Req::ok(
                &tok,
                body(
                    "echo",
                    "line",
                    r#","path":"src/a.rs","lines":{"start":2,"end":3}"#,
                ),
            ),
        )
        .await;
        assert_eq!(
            (s, &res["ok"]),
            (200, &serde_json::Value::Bool(true)),
            "{res}"
        );

        // 16. timeout: killed, its grandchild too.
        let (s, res) = send(&on, Req::ok(&tok, body("slow", "page", ""))).await;
        assert_eq!(s, 200);
        assert_eq!(res["ok"], false);
        assert!(
            res["first_line"]
                .as_str()
                .expect("line")
                .contains("timed out")
        );
        let pid = std::fs::read_to_string(cfg.path().join("grandchild.pid")).expect("pid");
        std::thread::sleep(std::time::Duration::from_millis(100));
        let alive = Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .expect("kill")
            .success();
        assert!(!alive, "grandchild survived");

        // 17. the 5th concurrent run is refused.
        let mut runs = Vec::new();
        for id in ["nap1", "nap2", "nap3", "nap4"] {
            let (r, t) = (on.clone(), tok.clone());
            runs.push(tokio::spawn(async move {
                send(&r, Req::ok(&t, body(id, "page", ""))).await.0
            }));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let (s, res) = send(&on, Req::ok(&tok, body("echo", "page", ""))).await;
        assert_eq!(s, 429, "5th concurrent: {res}");
        for r in runs {
            assert_eq!(r.await.expect("join"), 200);
        }

        // 18. the repo's own actions.toml files never load: naming one is refused.
        for local in [repo.clone(), repo.join(".gitterrace")] {
            let r = crate::build_multi(
                vec![repo.clone()],
                None,
                Some(&local.join("actions.toml")),
                &crate::app::Env::new(gitterrace_config::Config::default()),
            )
            .expect("router")
            .0;
            assert_eq!(
                send(&r, Req::ok("x", body("echo", "page", ""))).await.0,
                404,
                "{}",
                local.display()
            );
        }
        // 19. group-writable config: refused, no route.
        let gw = tempfile::tempdir().expect("tmp");
        write_config(gw.path(), ECHO, 0o620);
        let r = router(&repo, Some(gw.path()), None);
        assert_eq!(
            send(&r, Req::ok("x", body("echo", "page", ""))).await.0,
            404,
            "group writable"
        );
    }

    /// Repo content shows up escaped on every page that carries the action menu.
    #[tokio::test(flavor = "multi_thread")]
    async fn hostile_repo_content_is_escaped_where_the_menu_lives() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (repo, sha) = hostile(tmp.path());
        let cfg = config_dir(ECHO);
        let r = router(&repo, Some(cfg.path()), None);
        let branch = "x%22%3E%3Csvg-onload%3Dalert%283%29%3E";
        for uri in [
            "/".to_owned(),
            format!("/commit/{sha}"),
            "/blob/main/src/a.rs".to_owned(),
            "/blame/main/src/a.rs".to_owned(),
            "/tree/main/src".to_owned(),
            format!("/log/{branch}"),
            "/refs".to_owned(),
            "/search?q=script".to_owned(),
            "/stats/hotspots".to_owned(),
            format!("/tree/{branch}"),
        ] {
            let (s, html) = page(&r, &uri).await;
            assert!(s == 200 || s == 404, "{uri}: {s}");
            for raw in [
                "<script>alert(1)",
                "<img src=x onerror",
                "\"><img onerror",
                "\"><svg-onload",
            ] {
                assert!(!html.contains(raw), "{uri} renders {raw:?} unescaped");
            }
            if s == 200 {
                assert!(
                    html.contains("name=\"gitterrace-actions\""),
                    "{uri} lacks the menu token"
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn go_redirects_to_the_repo_holding_the_commit() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (repo, sha) = hostile(tmp.path());
        let id = gitterrace_git::Repo::discover(&repo)
            .expect("repo")
            .id()
            .expect("id");
        let r = router(&repo, None, None);
        let loc = |res: &axum::response::Response| {
            res.headers()
                .get("location")
                .map(|v| v.to_str().expect("ascii").to_owned())
        };
        for (uri, want) in [
            (format!("/go/{id}/{sha}"), Some(format!("/commit/{sha}"))),
            (
                format!("/go/{id}/{sha}/src/a.rs"),
                Some(format!("/blob/{sha}/src/a.rs")),
            ),
            (
                format!("/go/{id}/{sha}/src"),
                Some(format!("/tree/{sha}/src")),
            ),
            (format!("/go/{id}/{sha}/../../etc/passwd"), None),
            (format!("/go/000000000000/{sha}"), None),
            (format!("/go/{id}/{}", &sha[..12]), None),
        ] {
            let res = r
                .clone()
                .oneshot(
                    Request::get(&uri)
                        .header("host", "localhost")
                        .body(Body::empty())
                        .expect("req"),
                )
                .await
                .expect("res");
            match want {
                Some(w) => assert_eq!((res.status().as_u16(), loc(&res)), (302, Some(w)), "{uri}"),
                None => assert_eq!(res.status(), 404, "{uri}"),
            }
        }
        let (s, html) = page(&r, "/go/%3Cimg%20src%3Dx%3E/abc").await;
        assert_eq!(s, 404);
        assert!(html.contains("&lt;img src=x&gt;") && !html.contains("<img src=x>"));
        let (_, html) = page(&r, "/").await;
        assert!(
            html.contains(&format!("data-id=\"{id}\"")),
            "permalink id not in the page"
        );
    }

    /// Targets come back as parts with `var` already resolved; no file or a file with no
    /// `[[send]]` is `[]`, and sending needs no action.
    #[tokio::test(flavor = "multi_thread")]
    async fn send_targets_are_template_parts() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (repo, _) = hostile(tmp.path());
        let (status, body) = page(&router(&repo, None, None), "/_send/targets").await;
        assert_eq!((status, body.as_str()), (200, "[]"));
        let cfg = config_dir(
            "version = 1\n[[send]]\nid = \"gh\"\nlabel = \"New issue\"\nurl = \"https://github.com/{var.repo}/issues/new?title={title}&body={notes}\"\nvars = { repo = \"o/n\" }\n",
        );
        let r = router(&repo, Some(cfg.path()), None);
        let (status, body) = page(&r, "/_send/targets").await;
        assert_eq!(status, 200, "{body}");
        let want = serde_json::json!([{
            "id": "gh",
            "label": "New issue",
            "parts": [
                {"lit": "https://github.com/o/n/issues/new?title="},
                {"ph": "title"},
                {"lit": "&body="},
                {"ph": "notes"},
            ],
        }]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).expect("json"),
            want
        );
    }
}
