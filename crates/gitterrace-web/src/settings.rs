//! The settings routes: read, check and save `config.toml` and `actions.toml` from the browser.
//!
//! They exist only on a local server: a loopback bind, not public, with a config folder, never
//! in an export. Elsewhere the routes are not mounted, and `public.rs` lists them as local so a
//! public server answers 404 even if one were. Every request passes the same gate as the run
//! route (same-origin fetch metadata, matching `Origin`, the per-process token), reads included.
//! A request names a file from a closed set, never a path. A save carries the stamp of the text
//! it started from, so a tab never overwrites an edit made in an editor meanwhile.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    extract::{Query, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use gitterrace_actions::Token;
use gitterrace_config::{Change, Located, MAX_FILE_BYTES, SaveError, Stamp};
use gitterrace_types::SettingsFile;

use crate::app::Env;

/// What the routes need: where the two files are and the token pages echo back.
pub(crate) struct Settings {
    pub(crate) dir: PathBuf,
    /// The `actions.toml` the server was started with, `None` with actions off.
    pub(crate) actions: Option<PathBuf>,
    /// Whether the run route is mounted. Without it a first action needs a restart: the route
    /// and its token are decided once at startup, see `actions::start`.
    pub(crate) actions_on: bool,
    pub(crate) token: Token,
    pub(crate) env: Env,
}

impl Settings {
    pub(crate) fn token(&self) -> &str {
        self.token.as_str()
    }

    /// Where file `f` lives; `None` for actions.toml with actions off.
    pub(crate) fn path(&self, f: SettingsFile) -> Option<PathBuf> {
        match f {
            SettingsFile::Config => Some(self.dir.join("config.toml")),
            SettingsFile::Actions => self.actions.clone(),
        }
    }

    /// Adds `actions` to the restart list when `text` declares an action this server, started
    /// without one, cannot turn on live.
    fn note_restart(&self, f: SettingsFile, text: &str, reply: &mut CheckReply) {
        if f != SettingsFile::Actions || self.actions_on {
            return;
        }
        let home = gitterrace_config::home_var().map_or_else(PathBuf::new, PathBuf::from);
        if gitterrace_actions::Config::parse(text, &home).is_ok_and(|c| !c.actions.is_empty()) {
            reply.restart.push("actions");
        }
    }
}

#[cfg(test)]
thread_local! {
    /// The token of the last router built on this thread, for the tests to echo.
    static LAST_TOKEN: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
}

/// Mounted only on a local server; see the module comment.
pub(crate) fn router(s: Arc<Settings>) -> Router {
    #[cfg(test)]
    LAST_TOKEN.with(|t| *t.borrow_mut() = s.token().to_owned());
    Router::new()
        .route("/_settings", get(crate::settings_page::page))
        .route("/_settings/file", get(read).put(write))
        .route("/_settings/check", post(check))
        .with_state(s)
}

/// Whether `build_server` may mount the routes: never public, never on a bind other machines
/// reach, never without a config folder.
pub(crate) const fn mountable(public: bool, local: bool, has_dir: bool) -> bool {
    !public && local && has_dir
}

#[derive(serde::Deserialize)]
struct Which {
    f: SettingsFile,
}

impl CheckReply {
    /// A refused edit: just its one error.
    fn error(l: Located) -> Self {
        Self { errors: vec![l.into()], ..Self::default() }
    }
}

#[derive(serde::Serialize)]
struct FileReply {
    text: String,
    stamp: String,
    exists: bool,
}

#[derive(serde::Deserialize)]
struct Edit {
    f: SettingsFile,
    /// The whole new text (plain mode), or absent for a form edit.
    #[serde(default)]
    text: Option<String>,
    /// Form edits applied to the text on disk.
    #[serde(default)]
    changes: Vec<ChangeIn>,
    /// The stamp the text was read with: `none` for no file yet.
    #[serde(default)]
    stamp: Option<String>,
}

#[derive(serde::Deserialize)]
struct ChangeIn {
    id: String,
    /// A TOML literal, `None` to take the key out.
    #[serde(default)]
    value: Option<String>,
    /// For an actions row, the input as typed: the server makes the literal from the key's
    /// type in the actions schema, so the browser never writes TOML.
    #[serde(default)]
    input: Option<String>,
}

#[derive(serde::Serialize, Default)]
pub(crate) struct CheckReply {
    pub(crate) ok: bool,
    pub(crate) errors: Vec<ErrorOut>,
    warnings: Vec<String>,
    restart: Vec<&'static str>,
    /// The text the edit produces, so the plain editor shows what a form edit wrote.
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stamp: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct ErrorOut {
    pub(crate) line: usize,
    col: usize,
    key: Option<String>,
    pub(crate) msg: String,
}

impl From<Located> for ErrorOut {
    fn from(l: Located) -> Self {
        Self { line: l.line, col: l.col, key: l.key, msg: l.msg }
    }
}

fn json<T: serde::Serialize>(status: StatusCode, body: &T) -> Response {
    crate::guard::no_store((status, axum::Json(body)).into_response())
}

fn plain(status: StatusCode, why: &'static str) -> Response {
    (status, why).into_response()
}

fn gate(h: &HeaderMap, s: &Settings) -> Option<Response> {
    crate::guard::same_origin_write(h, &s.token).map(|why| plain(StatusCode::FORBIDDEN, why))
}

/// `GET /_settings/file?f=config|actions`: the text and its stamp.
async fn read(
    State(settings): State<Arc<Settings>>,
    headers: HeaderMap,
    Query(which): Query<Which>,
) -> Response {
    if let Some(no) = gate(&headers, &settings) {
        return no;
    }
    let f = which.f;
    let Some(path) = settings.path(f) else {
        return plain(StatusCode::NOT_FOUND, "actions are off\n");
    };
    let r = tokio::task::spawn_blocking(move || -> std::io::Result<FileReply> {
        let stamp = Stamp::of(&path)?;
        let text = match std::fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_file() && m.len() <= MAX_FILE_BYTES as u64 => {
                std::fs::read_to_string(&path)?
            }
            Ok(_) => return Err(std::io::Error::other("not a regular settings file")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        Ok(FileReply { text, exists: stamp.is_some(), stamp: Stamp::tag(stamp) })
    })
    .await;
    match r {
        Ok(Ok(body)) => json(StatusCode::OK, &body),
        Ok(Err(e)) => {
            tracing::warn!("settings: cannot read {f}: {e}");
            plain(StatusCode::CONFLICT, "the file cannot be read\n")
        }
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "read failed\n"),
    }
}

/// Reads the JSON body after the gate, bounded.
async fn body(s: &Settings, req: Request) -> Result<Edit, Box<Response>> {
    if let Some(no) = gate(req.headers(), s) {
        return Err(Box::new(no));
    }
    if !crate::guard::is_json(req.headers()) {
        return Err(Box::new(plain(StatusCode::UNSUPPORTED_MEDIA_TYPE, "json only\n")));
    }
    // The text plus JSON escaping and the change list.
    let limit = MAX_FILE_BYTES * 2;
    crate::guard::json_body(req, limit, || plain(StatusCode::BAD_REQUEST, "malformed\n")).await
}

/// The text an edit produces: the plain text as sent, or the form changes over the file on disk.
fn resulting_text(path: &std::path::Path, e: &Edit) -> Result<String, Located> {
    if let Some(t) = &e.text {
        return Ok(t.clone());
    }
    let current = std::fs::read_to_string(path).unwrap_or_default();
    let mut changes = Vec::new();
    for c in &e.changes {
        let value = match (&c.input, e.f) {
            (Some(input), SettingsFile::Actions) => {
                Some(crate::settings_actions::literal(&c.id, input).map_err(|msg| Located {
                    line: 0,
                    col: 0,
                    key: Some(c.id.clone()),
                    msg,
                })?)
            }
            _ => c.value.clone(),
        };
        changes.push(Change { id: c.id.clone(), value });
    }
    gitterrace_config::apply(&current, &changes)
}

/// Loads `text` as file `f` would be loaded: the errors with their lines, the warnings, and
/// for `config.toml` the restart keys it changes. `before` is the file as it is on disk: a key
/// the edit leaves alone is never named, even when a flag holds the running value elsewhere.
pub(crate) fn validate(f: SettingsFile, text: &str, before: &str, env: &Env) -> CheckReply {
    match f {
        SettingsFile::Config => match gitterrace_config::parse(text) {
            Ok(l) => CheckReply {
                ok: true,
                warnings: l.warnings,
                restart: gitterrace_config::parse(before)
                    .map_or_else(|_| (*env.cfg()).clone(), |b| b.config)
                    .restart_changes(&l.config),
                ..CheckReply::default()
            },
            Err(e) => CheckReply::error(gitterrace_config::located(text, &e)),
        },
        SettingsFile::Actions => match check_actions(text) {
            Ok(()) => CheckReply { ok: true, ..CheckReply::default() },
            Err(l) => CheckReply::error(l),
        },
    }
}

/// `actions.toml` through its own loader. Its errors carry no place, so a syntax error is placed
/// by the TOML parser first.
fn check_actions(text: &str) -> Result<(), Located> {
    gitterrace_config::syntax(text)?;
    let home = gitterrace_config::home_var().map_or_else(PathBuf::new, PathBuf::from);
    gitterrace_actions::Config::parse(text, &home).map(drop).map_err(|e| Located {
        line: 0,
        col: 0,
        key: None,
        msg: e.to_string(),
    })
}

/// `POST /_settings/check`: what a save of this edit would say, with nothing written.
async fn check(State(s): State<Arc<Settings>>, req: Request) -> Response {
    let e = match body(&s, req).await {
        Ok(e) => e,
        Err(no) => return *no,
    };
    let Some(path) = s.path(e.f) else {
        return plain(StatusCode::NOT_FOUND, "actions are off\n");
    };
    let env = s.env.clone();
    let r = tokio::task::spawn_blocking(move || match resulting_text(&path, &e) {
        Ok(text) => {
            let before = std::fs::read_to_string(&path).unwrap_or_default();
            let mut reply = validate(e.f, &text, &before, &env);
            s.note_restart(e.f, &text, &mut reply);
            reply.text = Some(text);
            reply
        }
        Err(l) => CheckReply::error(l),
    })
    .await;
    r.map_or_else(
        |_| plain(StatusCode::INTERNAL_SERVER_ERROR, "check failed\n"),
        |reply| json(StatusCode::OK, &reply),
    )
}

/// `PUT /_settings/file`: validates, then saves through `gitterrace_config::save`. The running
/// server picks the new file up the way it picks up an editor save.
async fn write(State(s): State<Arc<Settings>>, req: Request) -> Response {
    let e = match body(&s, req).await {
        Ok(e) => e,
        Err(no) => return *no,
    };
    let Some(path) = s.path(e.f) else {
        return plain(StatusCode::NOT_FOUND, "actions are off\n");
    };
    let Some(expect) = e.stamp.as_deref().and_then(Stamp::parse_tag) else {
        return plain(StatusCode::PRECONDITION_REQUIRED, "a save names the stamp it read\n");
    };
    let env = s.env.clone();
    let r = tokio::task::spawn_blocking(move || {
        let text = resulting_text(&path, &e).map_err(SaveError::Invalid)?;
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        let mut reply = validate(e.f, &text, &before, &env);
        s.note_restart(e.f, &text, &mut reply);
        let saved = gitterrace_config::save(&path, &text, expect, |t| match e.f {
            SettingsFile::Config => gitterrace_config::parse(t)
                .map(drop)
                .map_err(|err| gitterrace_config::located(t, &err)),
            SettingsFile::Actions => check_actions(t),
        })?;
        Ok::<_, SaveError>((reply, saved, e.f))
    })
    .await;
    match r {
        Ok(Ok((mut reply, saved, f))) => {
            tracing::info!("settings: {f} saved ({} restart keys changed)", reply.restart.len());
            reply.stamp = Some(Stamp::tag(Some(saved.stamp)));
            json(StatusCode::OK, &reply)
        }
        Ok(Err(SaveError::Invalid(l))) => {
            json(StatusCode::UNPROCESSABLE_ENTITY, &CheckReply::error(l))
        }
        Ok(Err(SaveError::Stale)) => {
            plain(StatusCode::CONFLICT, "changed on disk since it was read, reload it\n")
        }
        Ok(Err(SaveError::NotRegular(_))) => {
            plain(StatusCode::CONFLICT, "not a regular file, not written\n")
        }
        Ok(Err(SaveError::TooLarge)) => plain(StatusCode::PAYLOAD_TOO_LARGE, "too large\n"),
        Ok(Err(SaveError::Io(err))) => {
            tracing::warn!("settings: save failed: {err}");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "save failed\n")
        }
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "save failed\n"),
    }
}

#[cfg(test)]
mod tests;
