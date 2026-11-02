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
    response::{IntoResponse, Response},
    routing::{get, post},
};
use gitterrace_actions::{
    Action, Config, Limits, Output, Payload, PayloadError, Reloader, RunError, RunRequest, Runner,
    Token,
};
use gitterrace_git::Repo;

use crate::fmt::enc_path;

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

    /// The menu entries the client renders, see [`menu_entries`].
    pub(crate) fn menu_json(&self) -> String {
        menu_entries(&self.config())
    }
}

/// The one list every view's action menu is built from: every action in file order with its
/// label, shortcut, kinds and `why`, the tip a disabled entry shows. The page never drops an
/// entry: one the selection's kind does not fit stays in place, disabled, so the menu has the
/// same entries in the same order on every view.
pub(crate) fn menu_entries(config: &Config) -> String {
    let items: Vec<_> = config
        .actions
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "label": a.label,
                "kinds": a.kinds,
                "key": a.key.map(String::from),
                "why": why_off(&a.kinds),
            })
        })
        .collect();
    serde_json::Value::from(items).to_string()
}

/// "Works on lines or a commit": the tip of an entry the current selection does not fit.
fn why_off(kinds: &[gitterrace_actions::Kind]) -> String {
    use gitterrace_actions::Kind;
    let names: Vec<&str> = kinds
        .iter()
        .map(|k| match k {
            Kind::Line => "lines",
            Kind::File => "a file",
            Kind::Commit => "a commit",
            Kind::Hunk => "a hunk",
            Kind::Blame => "a blame line",
            Kind::SearchHit => "a search hit",
            Kind::Insight => "an insights row",
            Kind::Page => "a page",
        })
        .collect();
    let list = match names.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    };
    format!("Works on {list}")
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
    /// Follows the repositories added to and removed from the running server.
    pub(crate) targets: RwLock<Vec<Arc<Target>>>,
    pub(crate) live: Option<Arc<Live>>,
    pub(crate) status: Arc<Status>,
    /// The config `[[send]]` targets come from: the live, reloaded one when actions are on.
    pub(crate) sends: Arc<RwLock<Arc<Config>>>,
    /// The server's settings, read per request so a reload of `links.style` reaches payloads.
    pub(crate) env: crate::app::Env,
}

impl Site {
    /// The targets now, a copy so no lock is held across git reads.
    pub(crate) fn targets(&self) -> Vec<Arc<Target>> {
        self.targets
            .read()
            .map_or_else(|e| e.into_inner().clone(), |g| g.clone())
    }

    pub(crate) fn push_target(&self, t: Target) {
        let mut g = self
            .targets
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.push(Arc::new(t));
    }

    /// Drops the target of a repository taken off the server, so its permalinks stop resolving.
    pub(crate) fn drop_target(&self, key: &str) {
        let mut g = self
            .targets
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.retain(|t| t.key != key);
    }

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
        let targets = site.targets();
        let refs: Vec<&Target> = targets.iter().map(AsRef::as_ref).collect();
        let t = gitterrace_git::resolve_permalink(&refs, &id, &sha, path.as_deref())?;
        let url = match &path {
            None => format!("{}/commit/{sha}", t.base),
            Some(p) if t.repo.blob(&sha, p).is_ok() => {
                format!("{}/blob/{sha}/{}", t.base, enc_path(p))
            }
            Some(p) => format!("{}/tree/{sha}/{}", t.base, enc_path(p)),
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
    crate::error::not_found(&format!(
        "Repository {id} (or that commit) is not served here."
    ))
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
    let Some(t) = site
        .targets()
        .into_iter()
        .find(|t| t.key == rr.locator.repo)
    else {
        return bad("unknown repository");
    };
    let origin = format!("http://{host}");
    let result =
        tokio::task::spawn_blocking(move || run_blocking(&site, &t, &live, &action, &rr, &origin))
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
    t: &Target,
    live: &Live,
    action: &Action,
    rr: &RunRequest,
    origin: &str,
) -> Result<(gitterrace_actions::Outcome, Output), Failed> {
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
#[cfg(unix)]
pub(crate) mod actions_tests;

#[cfg(test)]
#[cfg(unix)]
#[expect(
    clippy::too_many_lines,
    reason = "one linear sequence, splitting it hides the flow"
)]
mod tests;

#[cfg(test)]
#[cfg(unix)]
mod menu_tests;
