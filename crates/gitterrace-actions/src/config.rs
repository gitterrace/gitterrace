//! `actions.toml`: the user's declared actions. Trust comes from where the file lives: the
//! user's config folder (or `--config <dir>`), never the served repository, its `.git`, the
//! cwd or an env var a repo could set. Running a click must never be a property of the
//! content being browsed.

use std::collections::{BTreeMap, HashSet};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

/// Default and ceiling for an action's timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TIMEOUT: Duration = Duration::from_secs(120);
/// A config larger than this is not hand-written; refuse instead of parsing it.
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// Limits on `[action.vars]`: names `[a-z0-9_]{1,32}`, values are plain strings.
const MAX_VARS: usize = 32;
const MAX_VAR_NAME: usize = 32;
const MAX_VAR_BYTES: usize = 4 * 1024;
/// Placeholders a `command` element may hold, besides `{var.NAME}`.
pub(crate) const FIELDS: [&str; 11] = [
    "title",
    "markdown",
    "text",
    "path",
    "line_start",
    "line_end",
    "sha",
    "url",
    "repo",
    "root",
    "kind",
];
/// Fields whose values come from the browsed repository. One of them opening an argv element
/// could be read as an option (`--upload-pack=...`), so the loader asks for a guard first.
pub(crate) const REPO_FIELDS: [&str; 4] = ["title", "markdown", "text", "path"];

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: {why}")]
    Unsafe { path: PathBuf, why: String },
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
    #[error("action {id}: {why}")]
    Invalid { id: String, why: String },
    #[error("script {id}: {why}")]
    Script { id: String, why: String },
    #[error("send {id}: {why}")]
    Send { id: String, why: String },
}

/// What a request may act on. Unknown names are a load error so typos surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Line,
    File,
    Commit,
    Hunk,
    Blame,
    SearchHit,
    Insight,
    Page,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cwd {
    Repo,
    Home,
    Path(PathBuf),
}

/// What the child reads on stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdin {
    /// The payload as JSON.
    Json,
    /// The `markdown` field alone, so `pbcopy` needs no script.
    Markdown,
    /// Nothing: stdin is closed.
    None,
}

/// Splits `name:modifier` into the placeholder name and its modifier, if any.
pub(crate) fn field_parts(raw: &str) -> (&str, Option<&str>) {
    match raw.split_once(':') {
        Some((name, m)) => (name, Some(m)),
        None => (raw, None),
    }
}

/// One piece of a `command` element: literal text or a `{name}` / `{name:modifier}` placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Field(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    Toast,
    Ignore,
}

/// One validated action. `argv` is a template: element 0 is a resolved, literal program and
/// every other element is expanded on its own, so a value is always exactly one argument and
/// never reaches a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub label: String,
    pub kinds: Vec<Kind>,
    pub argv: Vec<String>,
    pub stdin: Stdin,
    pub cwd: Cwd,
    pub timeout: Duration,
    pub output: Output,
    pub key: Option<char>,
    /// Extra variable names passed through from gitterrace's own environment.
    pub env: Vec<String>,
    /// Fixed values from `[action.vars]`, delivered as `GITTERRACE_VAR_<NAME>` and
    /// `payload.vars`, and to `{var.NAME}` in `argv`. Only `actions.toml` sets them.
    pub vars: BTreeMap<String, String>,
}

/// The loaded set, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub actions: Vec<Action>,
    /// `[[send]]` targets, in file order: URL templates the page opens, never a process.
    pub sends: Vec<crate::send::Target>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    version: u32,
    #[serde(default)]
    script: Vec<RawScript>,
    #[serde(default)]
    action: Vec<RawAction>,
    #[serde(default)]
    send: Vec<crate::send::RawSend>,
}

/// A `[[script]]` block: the command line written once for several actions. It cannot name
/// another script, so references are one level deep and never cycle.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScript {
    id: String,
    command: RawCommand,
    cwd: Option<String>,
    timeout: Option<String>,
    stdin: Option<String>,
    #[serde(default)]
    shell: bool,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum RawCommand {
    Argv(Vec<String>),
    Shell(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    id: String,
    label: String,
    kinds: Vec<Kind>,
    command: Option<RawCommand>,
    script: Option<String>,
    cwd: Option<String>,
    timeout: Option<String>,
    output: Option<Output>,
    stdin: Option<String>,
    shell: Option<bool>,
    key: Option<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    vars: BTreeMap<String, String>,
}

impl Config {
    pub fn get(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    /// Loads `path` after the ownership and permission checks. `repos` are the served
    /// repository roots: a config inside one is refused even when named by `--actions`.
    pub fn load(path: &Path, repos: &[PathBuf]) -> Result<Self, ConfigError> {
        check_file(path, repos)?;
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.into(), e))?;
        Self::parse(&text, &home())
    }

    /// Parses and validates; `argv[0]` is resolved against `home` and `PATH` now, once.
    pub fn parse(text: &str, home: &Path) -> Result<Self, ConfigError> {
        let raw: RawFile = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        if raw.version != 1 {
            return Err(ConfigError::Parse(format!(
                "unsupported version {}",
                raw.version
            )));
        }
        let mut scripts = BTreeMap::new();
        for sc in raw.script {
            if !valid_id(&sc.id) {
                return Err(script_err(&sc.id, "id must match [a-z0-9-]{1,32}"));
            }
            if scripts.contains_key(&sc.id) {
                return Err(script_err(&sc.id, "duplicate id"));
            }
            check_command(&sc.command, sc.shell, None).map_err(|why| script_err(&sc.id, why))?;
            scripts.insert(sc.id.clone(), sc);
        }
        let mut seen = HashSet::new();
        let mut actions = Vec::with_capacity(raw.action.len());
        for a in raw.action {
            let action = validate(a, &scripts, home)?;
            if !seen.insert(action.id.clone()) {
                return Err(invalid(&action.id, "duplicate id"));
            }
            actions.push(action);
        }
        let mut sends = Vec::with_capacity(raw.send.len());
        for raw_send in raw.send {
            let id = raw_send.id().to_owned();
            let target = crate::send::compile(raw_send).map_err(|why| send_err(&id, why))?;
            if !valid_id(&id) {
                return Err(send_err(&id, "id must match [a-z0-9-]{1,32}"));
            }
            if sends.iter().any(|t: &crate::send::Target| t.id == id) {
                return Err(send_err(&id, "duplicate id"));
            }
            sends.push(target);
        }
        Ok(Self { actions, sends })
    }

    /// Lines like "action added: copy -> ~/bin/copy.sh" for a reload log.
    pub fn diff(&self, new: &Self) -> Vec<String> {
        let old: BTreeMap<_, _> = self.actions.iter().map(|a| (&a.id, a)).collect();
        let next: BTreeMap<_, _> = new.actions.iter().map(|a| (&a.id, a)).collect();
        let mut out = Vec::new();
        for (id, a) in &next {
            match old.get(id) {
                None => out.push(format!("action added: {id} -> {}", a.argv.join(" "))),
                Some(o) if o != a => {
                    out.push(format!("action changed: {id} -> {}", a.argv.join(" ")));
                }
                Some(_) => {}
            }
        }
        out.extend(
            old.keys()
                .filter(|id| !next.contains_key(*id))
                .map(|id| format!("action removed: {id}")),
        );
        out
    }
}

/// `actions.toml` in gitterrace's config folder (`--config <dir>`, else the XDG folder). The
/// folder rule lives in `gitterrace-config`, so no environment variable a repo could set
/// moves it. `None` when there is no home to resolve against.
pub fn default_path(config_flag: Option<&Path>) -> Option<PathBuf> {
    gitterrace_config::config_dir(config_flag).map(|d| d.join("actions.toml"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn invalid(id: &str, why: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        id: id.to_string(),
        why: why.into(),
    }
}

fn script_err(id: &str, why: impl Into<String>) -> ConfigError {
    ConfigError::Script {
        id: id.to_string(),
        why: why.into(),
    }
}

fn send_err(id: &str, why: impl Into<String>) -> ConfigError {
    ConfigError::Send {
        id: id.to_string(),
        why: why.into(),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn unsafe_path(path: &Path, why: impl Into<String>) -> ConfigError {
    ConfigError::Unsafe {
        path: path.to_path_buf(),
        why: why.into(),
    }
}

/// Regular file (not a symlink), owned by us, not group or world writable; the same for
/// its directory; and not inside a served repository.
fn check_file(path: &Path, repos: &[PathBuf]) -> Result<(), ConfigError> {
    let uid = rustix::process::getuid().as_raw();
    let meta = std::fs::symlink_metadata(path).map_err(|e| ConfigError::Io(path.into(), e))?;
    if !meta.file_type().is_file() {
        return Err(unsafe_path(path, "not a regular file"));
    }
    if meta.len() > MAX_CONFIG_BYTES {
        return Err(unsafe_path(path, "too large"));
    }
    owned_private(path, &meta, uid)?;
    let real = path
        .canonicalize()
        .map_err(|e| ConfigError::Io(path.into(), e))?;
    let dir = real
        .parent()
        .ok_or_else(|| unsafe_path(path, "no parent dir"))?;
    let dmeta = std::fs::metadata(dir).map_err(|e| ConfigError::Io(dir.into(), e))?;
    owned_private(dir, &dmeta, uid)?;
    for root in repos {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        if real.starts_with(&root) {
            return Err(unsafe_path(path, "inside a served repository"));
        }
    }
    Ok(())
}

fn owned_private(path: &Path, meta: &std::fs::Metadata, uid: u32) -> Result<(), ConfigError> {
    if meta.uid() != uid {
        return Err(unsafe_path(path, "not owned by the current user"));
    }
    if meta.permissions().mode() & 0o022 != 0 {
        return Err(unsafe_path(path, "group or world writable"));
    }
    Ok(())
}

fn validate(
    a: RawAction,
    scripts: &BTreeMap<String, RawScript>,
    home: &Path,
) -> Result<Action, ConfigError> {
    let id = a.id;
    if !valid_id(&id) {
        return Err(invalid(&id, "id must match [a-z0-9-]{1,32}"));
    }
    if a.label.trim().is_empty() || a.label.chars().count() > 80 {
        return Err(invalid(&id, "label must be 1 to 80 characters"));
    }
    if a.kinds.is_empty() {
        return Err(invalid(&id, "kinds is empty"));
    }
    // The action's own command, or the named script's; its cwd and timeout override.
    let (command, shell, base_cwd, base_timeout, base_stdin) = match (a.script, a.command) {
        (Some(_), Some(_)) => return Err(invalid(&id, "set either script or command, not both")),
        (None, None) => return Err(invalid(&id, "command or script is required")),
        (None, Some(c)) => (c, a.shell.unwrap_or(false), None, None, None),
        (Some(name), None) => {
            if a.shell.is_some() {
                return Err(invalid(&id, "shell belongs to the script block"));
            }
            let sc = scripts
                .get(&name)
                .ok_or_else(|| invalid(&id, format!("script {name:?} is not defined")))?;
            (
                sc.command.clone(),
                sc.shell,
                sc.cwd.clone(),
                sc.timeout.clone(),
                sc.stdin.clone(),
            )
        }
    };
    check_command(&command, shell, Some(&a.vars)).map_err(|why| invalid(&id, why))?;
    let stdin = parse_stdin(a.stdin.or(base_stdin).as_deref()).map_err(|why| invalid(&id, why))?;
    let argv = match (shell, command) {
        (false, RawCommand::Argv(v)) if !v.is_empty() => {
            let mut v = v;
            v[0] = resolve_program(&v[0], home)
                .ok_or_else(|| invalid(&id, format!("{} is not an executable file", v[0])))?;
            v
        }
        (false, _) => {
            return Err(invalid(
                &id,
                "command must be a non-empty array when shell = false",
            ));
        }
        (true, RawCommand::Shell(s)) if !s.trim().is_empty() => {
            vec!["/bin/sh".into(), "-c".into(), s]
        }
        (true, _) => return Err(invalid(&id, "command must be one string when shell = true")),
    };
    let cwd = match a.cwd.or(base_cwd).as_deref() {
        None | Some("home") => Cwd::Home,
        Some("repo") => Cwd::Repo,
        Some(p) => {
            let p = expand_tilde(p, home);
            if !p.is_absolute() || !p.is_dir() {
                return Err(invalid(
                    &id,
                    "cwd must be repo, home or an existing absolute dir",
                ));
            }
            Cwd::Path(p)
        }
    };
    let timeout = match a.timeout.or(base_timeout).as_deref() {
        None => DEFAULT_TIMEOUT,
        Some(t) => parse_duration(t)
            .filter(|d| !d.is_zero() && *d <= MAX_TIMEOUT)
            .ok_or_else(|| invalid(&id, "timeout must be like 10s or 500ms, at most 120s"))?,
    };
    let key = parse_key(a.key.as_deref()).map_err(|why| invalid(&id, why))?;
    for name in &a.env {
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || name.starts_with("GITTERRACE_")
        {
            return Err(invalid(&id, format!("env name {name:?} is not allowed")));
        }
    }
    let vars = validate_vars(&id, a.vars)?;
    Ok(Action {
        id,
        label: a.label,
        kinds: a.kinds,
        argv,
        stdin,
        cwd,
        timeout,
        output: a.output.unwrap_or(Output::Toast),
        key,
        env: a.env,
        vars,
    })
}

/// Splits a `command` element into literal text and `{name}` placeholders. `{{` and `}}` are
/// literal braces; a lone brace is an error so a typo never passes as text.
pub fn pieces(element: &str) -> Result<Vec<Piece>, String> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut chars = element.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c);
                }
                if !closed {
                    return Err(format!("{element:?}: unclosed {{"));
                }
                if name.is_empty() || name.contains('{') {
                    return Err(format!(
                        "{element:?}: write a literal brace as {{{{ or }}}}"
                    ));
                }
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Field(name));
            }
            '}' => {
                return Err(format!(
                    "{element:?}: write a literal brace as {{{{ or }}}}"
                ));
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    Ok(out)
}

/// The placeholder rules, checked once at load. `vars` is the action's `[action.vars]`, or
/// `None` for a `[[script]]` block, whose `{var.NAME}` is checked against each action using it.
fn check_command(
    command: &RawCommand,
    shell: bool,
    vars: Option<&BTreeMap<String, String>>,
) -> Result<(), String> {
    let known = |name: &str| match name.strip_prefix("var.") {
        Some(v) => vars.is_none_or(|vars| vars.contains_key(v)),
        None => FIELDS.contains(&name),
    };
    let elements = match command {
        RawCommand::Shell(s) => {
            // `${HOME}` and awk's `{print}` are shell text; only a real placeholder name is
            // refused, so existing shell commands keep working.
            let named = FIELDS
                .iter()
                .map(|f| format!("{{{f}}}"))
                .chain(std::iter::once("{var.".to_string()))
                .find(|p| s.contains(p.as_str()));
            if shell && let Some(p) = named {
                return Err(format!(
                    "command: shell = true cannot take placeholders ({p}); write command as an \
                     array and drop shell"
                ));
            }
            return Ok(());
        }
        RawCommand::Argv(v) => v,
    };
    let mut after_dashdash = false;
    let mut prev_option = false;
    for (i, el) in elements.iter().enumerate() {
        let ps = pieces(el).map_err(|why| format!("command[{i}]: {why}"))?;
        for p in &ps {
            if let Piece::Field(raw) = p {
                let (name, modifier) = field_parts(raw);
                if let Some(m) = modifier
                    && !crate::schema::MODIFIERS.iter().any(|(k, _)| *k == m)
                {
                    let known: Vec<_> = crate::schema::MODIFIERS.iter().map(|(k, _)| *k).collect();
                    return Err(format!(
                        "command[{i}]: {{{raw}}}: unknown modifier :{m}; known: :{}",
                        known.join(", :")
                    ));
                }
                if !known(name) {
                    return Err(match name.strip_prefix("var.") {
                        Some(v) => {
                            format!("command[{i}]: {{{name}}}: no var {v:?} in [action.vars]")
                        }
                        None => format!(
                            "command[{i}]: unknown placeholder {{{name}}}; known: {}, var.NAME",
                            FIELDS.join(", ")
                        ),
                    });
                }
                if i == 0 {
                    return Err(format!(
                        "command[0]: the program is literal, {{{raw}}} is not allowed there"
                    ));
                }
            }
        }
        let opens_with_repo_value = matches!(ps.first(),
            Some(Piece::Field(n)) if REPO_FIELDS.contains(&field_parts(n).0));
        // A value right after a literal option (`--notes {markdown}`) is that option's
        // argument, so the parser never reads it as an option.
        if opens_with_repo_value && !after_dashdash && !prev_option {
            return Err(format!(
                "command[{i}]: {el} comes from the repository and could start with `-`; put a \
                 literal \"--\" before it, or give it a literal prefix (\"--flag={el}\")"
            ));
        }
        after_dashdash |= el == "--";
        prev_option = i > 0 && el.starts_with('-') && el != "--" && !el.contains('{');
    }
    Ok(())
}

fn parse_key(key: Option<&str>) -> Result<Option<char>, &'static str> {
    let Some(k) = key else { return Ok(None) };
    let mut c = k.chars();
    match (c.next(), c.next()) {
        (Some(ch), None) if ch.is_ascii_alphanumeric() => Ok(Some(ch)),
        _ => Err("key must be one letter or digit"),
    }
}

fn parse_stdin(value: Option<&str>) -> Result<Stdin, String> {
    match value {
        None | Some("json") => Ok(Stdin::Json),
        Some("markdown") => Ok(Stdin::Markdown),
        Some("none") => Ok(Stdin::None),
        Some(other) => Err(format!("stdin = {other:?}: use json, markdown or none")),
    }
}

fn validate_vars(
    id: &str,
    vars: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ConfigError> {
    if vars.len() > MAX_VARS {
        return Err(invalid(id, format!("at most {MAX_VARS} vars")));
    }
    for (name, value) in &vars {
        let lower = name.to_ascii_lowercase();
        let fixable = lower.len() <= MAX_VAR_NAME
            && lower
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if *name != lower && fixable {
            return Err(invalid(id, format!("vars.{name}: use lowercase: {lower}")));
        }
        if name.is_empty()
            || name.len() > MAX_VAR_NAME
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(invalid(
                id,
                format!("vars.{name}: name must match [a-z0-9_]{{1,32}}"),
            ));
        }
        if value.len() > MAX_VAR_BYTES {
            return Err(invalid(id, format!("vars.{name}: value over 4 KiB")));
        }
        if value.contains('\0') {
            return Err(invalid(id, format!("vars.{name}: value contains NUL")));
        }
    }
    Ok(vars)
}

fn expand_tilde(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

/// argv[0] as an absolute executable: tilde-expanded, or looked up in `PATH` when it has no
/// slash. A relative path with a slash is refused: it would resolve against the cwd.
fn resolve_program(prog: &str, home: &Path) -> Option<String> {
    let exec = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let found = if prog.contains('/') || prog.starts_with('~') {
        Some(expand_tilde(prog, home)).filter(|p| p.is_absolute() && exec(p))
    } else {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .filter(|d| d.is_absolute())
            .map(|d| d.join(prog))
            .find(|p| exec(p))
    };
    found.map(|p| p.to_string_lossy().into_owned())
}

fn parse_duration(t: &str) -> Option<Duration> {
    if let Some(ms) = t.strip_suffix("ms") {
        return ms.parse().ok().map(Duration::from_millis);
    }
    t.strip_suffix('s')?.parse().ok().map(Duration::from_secs)
}

/// Holds the live set and reloads it when the file changes. Mounting the route is a startup
/// decision made by the caller; a reload only swaps the set, never mints a token.
pub struct Reloader {
    path: PathBuf,
    repos: Vec<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    current: Arc<RwLock<Arc<Config>>>,
    error: Option<String>,
}

impl Reloader {
    pub fn new(path: PathBuf, repos: Vec<PathBuf>, initial: Config) -> Self {
        let stamp = stamp(&path);
        Self {
            path,
            repos,
            stamp,
            current: Arc::new(RwLock::new(Arc::new(initial))),
            error: None,
        }
    }

    /// Why the last reload was refused; `None` once a later one succeeded.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The shared handle the run route reads from.
    pub fn handle(&self) -> Arc<RwLock<Arc<Config>>> {
        Arc::clone(&self.current)
    }

    /// One poll: when the file changed, a valid file replaces the set and an invalid one
    /// keeps it. Returns the log lines.
    pub fn poll(&mut self) -> Vec<String> {
        let now = stamp(&self.path);
        if now == self.stamp {
            return Vec::new();
        }
        self.stamp = now;
        match Config::load(&self.path, &self.repos) {
            Ok(new) => {
                self.error = None;
                let Ok(mut cur) = self.current.write() else {
                    return vec!["actions: config lock poisoned, reload skipped".into()];
                };
                let lines = cur.diff(&new);
                *cur = Arc::new(new);
                lines
            }
            Err(e) => {
                self.error = Some(e.to_string());
                vec![format!("actions: reload refused, keeping the old set: {e}")]
            }
        }
    }
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::symlink_metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use crate::config::{Config, ConfigError, Cwd, Kind, Reloader, default_path};

    fn private_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
        d
    }

    fn write(dir: &Path, name: &str, body: &str, mode: u32) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("chmod");
        p
    }

    const GOOD: &str = r#"
version = 1
[[action]]
id = "echo"
label = "Echo"
kinds = ["line", "commit"]
command = ["/bin/echo", "hi"]
cwd = "repo"
timeout = "3s"
key = "e"

[[action]]
id = "sh"
label = "Shell"
kinds = ["page"]
shell = true
command = 'printf %s "$GITTERRACE_TITLE"'
"#;

    #[test]
    fn parses_a_good_file() {
        let c = Config::parse(GOOD, Path::new("/home/x")).expect("parse");
        let echo = c.get("echo").expect("echo");
        assert_eq!(echo.argv, ["/bin/echo", "hi"]);
        assert_eq!(echo.cwd, Cwd::Repo);
        assert_eq!(echo.timeout.as_secs(), 3);
        assert_eq!(echo.kinds, [Kind::Line, Kind::Commit]);
        assert_eq!(c.get("sh").expect("sh").argv[..2], ["/bin/sh", "-c"]);
    }

    #[test]
    fn refuses_bad_actions() {
        let base = |extra: &str| {
            format!(
                "version = 1\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\n{extra}\n"
            )
        };
        for bad in [
            "command = [\"/bin/echo\"]\nkinds2 = 1",
            "command = []",
            "command = \"/bin/echo hi\"",
            "command = [\"/bin/echo\"]\nshell = true",
            "command = [\"./relative\"]",
            "command = [\"/nonexistent/prog\"]",
            "command = [\"/bin/echo\"]\ntimeout = \"121s\"",
            "command = [\"/bin/echo\"]\ntimeout = \"soon\"",
            "command = [\"/bin/echo\"]\ncwd = \"relative\"",
            "command = [\"/bin/echo\"]\nkey = \"ab\"",
            "command = [\"/bin/echo\"]\nenv = [\"GITTERRACE_X\"]",
        ] {
            assert!(Config::parse(&base(bad), Path::new("/")).is_err(), "{bad}");
        }
        let typo = GOOD.replace("\"commit\"", "\"comit\"");
        assert!(
            Config::parse(&typo, Path::new("/")).is_err(),
            "unknown kind"
        );
        let dup = format!(
            "{GOOD}\n[[action]]\nid = \"echo\"\nlabel = \"x\"\nkinds = [\"page\"]\ncommand = [\"/bin/echo\"]\n"
        );
        assert!(matches!(
            Config::parse(&dup, Path::new("/")),
            Err(ConfigError::Invalid { .. })
        ));
        assert!(
            Config::parse(&GOOD.replace("version = 1", "version = 2"), Path::new("/")).is_err()
        );
        let upper = GOOD.replace("id = \"echo\"", "id = \"Echo\"");
        assert!(Config::parse(&upper, Path::new("/")).is_err());
    }

    fn err(text: &str) -> String {
        Config::parse(text, Path::new("/"))
            .expect_err("must refuse")
            .to_string()
    }

    const HEAD: &str = "version = 1\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\ncommand = [\"/bin/echo\"]\n";

    #[test]
    fn vars_are_validated_naming_the_key() {
        let ok = Config::parse(
            &format!("{HEAD}[action.vars]\nvault = \"Notes\"\nmode_2 = \"daily\"\n"),
            Path::new("/"),
        )
        .expect("parse");
        let vars = &ok.get("a").expect("a").vars;
        assert_eq!(vars.get("vault").map(String::as_str), Some("Notes"));
        assert_eq!(vars.len(), 2);

        for (bad, needle) in [
            ("MODE = \"x\"", "use lowercase: mode"),
            ("\"a-b\" = \"x\"", "vars.a-b"),
            ("\"\" = \"x\"", "vars."),
            (&format!("{} = \"x\"", "n".repeat(33)), "name must match"),
            (&format!("big = \"{}\"", "x".repeat(4097)), "vars.big"),
            ("num = 3", "parse"),
        ] {
            let e = err(&format!("{HEAD}[action.vars]\n{bad}\n"));
            assert!(
                e.contains(needle) && e.contains("action a") || needle == "parse",
                "{bad}: {e}"
            );
        }
        let at_limit = format!("{HEAD}[action.vars]\nbig = \"{}\"\n", "x".repeat(4096));
        assert!(Config::parse(&at_limit, Path::new("/")).is_ok());
        let many: String = (0..33)
            .map(|i| ["v", &i.to_string(), " = \"x\"\n"].concat())
            .collect();
        assert!(err(&format!("{HEAD}[action.vars]\n{many}")).contains("at most 32 vars"));
    }

    #[test]
    fn scripts_are_shared_and_overridden() {
        let text = r#"
version = 1
[[script]]
id = "obsidian"
command = ["/bin/echo", "obs"]
cwd = "repo"
timeout = "5s"

[[action]]
id = "daily"
label = "Daily"
kinds = ["page"]
script = "obsidian"
timeout = "7s"
[action.vars]
mode = "daily"

[[action]]
id = "inbox"
label = "Inbox"
kinds = ["page"]
script = "obsidian"
cwd = "home"
"#;
        let c = Config::parse(text, Path::new("/")).expect("parse");
        let daily = c.get("daily").expect("daily");
        let inbox = c.get("inbox").expect("inbox");
        assert_eq!(daily.argv, ["/bin/echo", "obs"]);
        assert_eq!(inbox.argv, daily.argv);
        assert_eq!((daily.cwd.clone(), daily.timeout.as_secs()), (Cwd::Repo, 7));
        assert_eq!((inbox.cwd.clone(), inbox.timeout.as_secs()), (Cwd::Home, 5));

        let e = err(&text.replace("script = \"obsidian\"\ncwd", "script = \"nope\"\ncwd"));
        assert!(e.contains("action inbox") && e.contains("\"nope\""), "{e}");
        let both = err(&format!("{HEAD}script = \"x\"\n"));
        assert!(both.contains("not both"), "{both}");
        let dup = err(&format!(
            "{text}\n[[script]]\nid = \"obsidian\"\ncommand = [\"/bin/echo\"]\n"
        ));
        assert!(
            dup.contains("script obsidian") && dup.contains("duplicate"),
            "{dup}"
        );
        // A script cannot reference another script, so a cycle cannot be written.
        let nested =
            err("version = 1\n[[script]]\nid = \"s\"\nscript = \"s\"\ncommand = [\"/bin/echo\"]\n");
        assert!(nested.contains("script"), "{nested}");
        let bad_cmd = err(
            "version = 1\n[[script]]\nid = \"s\"\ncommand = [\"/nonexistent\"]\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\nscript = \"s\"\n",
        );
        assert!(bad_cmd.contains("action a"), "{bad_cmd}");
    }

    #[test]
    fn permission_checks() {
        let d = private_dir();
        let ok = write(d.path(), "actions.toml", GOOD, 0o600);
        assert_eq!(Config::load(&ok, &[]).expect("load").actions.len(), 2);

        let gw = write(d.path(), "gw.toml", GOOD, 0o620);
        assert!(
            matches!(Config::load(&gw, &[]), Err(ConfigError::Unsafe { .. })),
            "group writable"
        );
        let ww = write(d.path(), "ww.toml", GOOD, 0o602);
        assert!(
            matches!(Config::load(&ww, &[]), Err(ConfigError::Unsafe { .. })),
            "world writable"
        );

        let link = d.path().join("link.toml");
        std::os::unix::fs::symlink(&ok, &link).expect("symlink");
        assert!(
            matches!(Config::load(&link, &[]), Err(ConfigError::Unsafe { .. })),
            "symlink"
        );

        let open = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(open.path(), std::fs::Permissions::from_mode(0o777))
            .expect("chmod");
        let in_open = write(open.path(), "actions.toml", GOOD, 0o600);
        assert!(
            matches!(Config::load(&in_open, &[]), Err(ConfigError::Unsafe { .. })),
            "writable parent"
        );
    }

    #[test]
    fn repo_local_config_is_never_loaded() {
        let repo = private_dir();
        write(repo.path(), "actions.toml", GOOD, 0o600);
        std::fs::create_dir_all(repo.path().join(".gitterrace")).expect("mkdir");
        write(
            &repo.path().join(".gitterrace"),
            "actions.toml",
            GOOD,
            0o600,
        );
        let roots = vec![repo.path().to_path_buf()];
        // The default path is the user's config dir, never relative to a repo or the cwd.
        let default = default_path(None).expect("home");
        assert!(default.is_absolute());
        assert!(!default.starts_with(repo.path()));
        // Even named explicitly, a file inside a served repo is refused.
        for p in [
            repo.path().join("actions.toml"),
            repo.path().join(".gitterrace/actions.toml"),
        ] {
            assert!(
                matches!(Config::load(&p, &roots), Err(ConfigError::Unsafe { .. })),
                "{}",
                p.display()
            );
        }
    }

    #[test]
    fn reload_swaps_valid_and_keeps_old_on_invalid() {
        let d = private_dir();
        let p = write(d.path(), "actions.toml", GOOD, 0o600);
        let mut r = Reloader::new(p.clone(), vec![], Config::load(&p, &[]).expect("load"));
        let h = r.handle();
        assert!(r.poll().is_empty(), "unchanged file is not reloaded");

        let fewer = GOOD
            .split("[[action]]")
            .take(2)
            .collect::<Vec<_>>()
            .join("[[action]]")
            + "\n# x";
        write(d.path(), "actions.toml", &fewer, 0o600);
        assert_eq!(r.poll(), ["action removed: sh"]);
        assert_eq!(h.read().expect("lock").actions.len(), 1);

        write(
            d.path(),
            "actions.toml",
            "version = 1\n[[action]]\nbroken",
            0o600,
        );
        assert!(r.poll()[0].contains("keeping the old set"));
        assert_eq!(h.read().expect("lock").actions.len(), 1);

        let with_vars = format!("{fewer}\n[action.vars]\nmode = \"daily\"\n");
        write(d.path(), "actions.toml", &with_vars, 0o600);
        assert_eq!(r.poll(), ["action changed: echo -> /bin/echo hi"]);
        let bad_var = with_vars.replace("mode =", "Mode =") + "\n";
        write(d.path(), "actions.toml", &bad_var, 0o600);
        assert!(r.poll()[0].contains("keeping the old set"));
        let cur = h.read().expect("lock").clone();
        assert_eq!(
            cur.get("echo")
                .expect("echo")
                .vars
                .get("mode")
                .map(String::as_str),
            Some("daily")
        );
    }

    #[test]
    fn placeholders_load_and_every_refusal_names_its_key() {
        let cmd = |c: &str| HEAD.replace("command = [\"/bin/echo\"]", c);
        let ok = Config::parse(
            &cmd("command = [\"/bin/echo\", \"--notes\", \"{markdown}\", \"--\", \"{title}\", \"{{x}}\"]\nstdin = \"markdown\""),
            Path::new("/"),
        )
        .expect("parse");
        let a = ok.get("a").expect("a");
        assert_eq!(
            a.argv[1..],
            ["--notes", "{markdown}", "--", "{title}", "{{x}}"]
        );
        assert_eq!(a.stdin, crate::config::Stdin::Markdown);
        // A literal prefix or a trusted first value needs no `--`.
        let prefixed = cmd("command = [\"/bin/echo\", \"x={title}\", \"{root}/{path}\"]");
        let url = cmd(
            "command = [\"/bin/echo\", \"tendir://new-task?title={title:url}&notes={markdown:url}\", \"{var.v:url}\", \"{sha:url}\"]\nvars = { v = \"x\" }",
        );
        if let Err(e) = Config::parse(&url, Path::new("/")) {
            panic!("{e}");
        }
        assert!(Config::parse(&prefixed, Path::new("/")).is_ok());
        for (bad, needle) in [
            (
                "command = [\"/bin/echo\", \"{titel}\"]",
                "command[1]: unknown placeholder {titel}",
            ),
            (
                "command = [\"{sha}\"]",
                "command[0]: the program is literal",
            ),
            (
                "command = [\"/bin/echo\", \"{title}\"]",
                "command[1]: {title} comes from the repository",
            ),
            (
                "command = [\"/bin/echo\", \"{path}\", \"--\"]",
                "put a literal \"--\"",
            ),
            (
                "command = [\"/bin/echo\", \"--\", \"{title:bogus}\"]",
                "command[2]: {title:bogus}: unknown modifier :bogus; known: :url",
            ),
            (
                "command = [\"/bin/echo\", \"{title:url}\"]",
                "command[1]: {title:url} comes from the repository",
            ),
            (
                "command = [\"/bin/echo\", \"--\", \"{titel:url}\"]",
                "command[2]: unknown placeholder {titel}",
            ),
            (
                "command = [\"/bin/echo\", \"{var.nope}\"]",
                "command[1]: {var.nope}: no var",
            ),
            ("command = [\"/bin/echo\", \"a}\"]", "command[1]"),
            ("command = [\"/bin/echo\", \"{x\"]", "command[1]"),
            (
                "shell = true\ncommand = 'echo {title}'",
                "command: shell = true cannot take placeholders",
            ),
            (
                "command = [\"/bin/echo\"]\nstdin = \"xml\"",
                "stdin = \"xml\"",
            ),
        ] {
            let e = err(&cmd(bad));
            assert!(e.contains("action a") && e.contains(needle), "{bad}: {e}");
        }
        let script =
            "version = 1\n[[script]]\nid = \"s\"\ncommand = [\"/bin/echo\", \"{title}\"]\n";
        let e = err(script);
        assert!(e.contains("script s") && e.contains("command[1]"), "{e}");
        let shell = "version = 1\n[[script]]\nid = \"s\"\nshell = true\ncommand = 'echo {url}'\n";
        assert!(
            err(shell).contains("script s: command: shell"),
            "{}",
            err(shell)
        );
        // A script's {var.NAME} is checked against each action that uses it.
        let uses = "version = 1\n[[script]]\nid = \"s\"\ncommand = [\"/bin/echo\", \"{var.url}\"]\n\
                    [[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\nscript = \"s\"\n";
        assert!(
            err(uses).contains("action a: command[1]: {var.url}"),
            "{}",
            err(uses)
        );
        let with = format!("{uses}[action.vars]\nurl = \"https://x\"\n");
        assert!(Config::parse(&with, Path::new("/")).is_ok());
        // Shell text that only looks like braces keeps working.
        let awk = cmd("shell = true\ncommand = 'echo ${HOME} | awk \"{print}\"'");
        assert!(Config::parse(&awk, Path::new("/")).is_ok());
    }

    /// Every `toml` block in docs/actions.md loads, so the docs never show a config the
    /// server refuses. Programs are stubbed: `~/...` under a temp home, bare names as
    /// absolute stubs, so only the PATH lookup itself is not exercised here.
    #[test]
    fn every_documented_example_loads() {
        use std::os::unix::fs::PermissionsExt;
        let doc = include_str!("../../../docs/actions.md");
        let home = tempfile::tempdir().expect("tmp");
        let stub = |p: &Path| {
            std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
            std::fs::write(p, "#!/bin/sh\n").expect("write");
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        };
        let blocks: Vec<&str> = doc
            .split("```toml\n")
            .skip(1)
            .filter_map(|b| b.split("```").next())
            .collect();
        assert!(blocks.len() >= 3, "found {} toml blocks", blocks.len());
        for (i, block) in blocks.iter().enumerate() {
            let mut text = if block.contains("version") {
                (*block).to_owned()
            } else {
                format!("version = 1\n{block}")
            };
            for prog in text
                .lines()
                .filter_map(|l| l.trim().strip_prefix("command"))
                .filter_map(|l| l.split('"').nth(1))
                .map(str::to_owned)
                .collect::<Vec<_>>()
            {
                if let Some(rest) = prog.strip_prefix("~/") {
                    stub(&home.path().join(rest));
                } else if !prog.contains('/') {
                    let abs = home.path().join("bin").join(&prog);
                    stub(&abs);
                    text =
                        text.replace(&format!("[\"{prog}\""), &format!("[\"{}\"", abs.display()));
                }
            }
            if let Err(e) = Config::parse(&text, home.path()) {
                panic!("docs/actions.md toml block {i} does not load: {e}\n{text}");
            }
        }
    }
}
