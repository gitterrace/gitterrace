//! `actions.toml`: the user's declared actions.
//!
//! Trust comes from where the file lives: the
//! user's config folder (or `--config <dir>`), never the served repository, its `.git`, the
//! cwd or an env var a repo could set. Running a click must never be a property of the
//! content being browsed.

mod command;
mod program;
mod reload;
mod trust;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

pub use crate::config::command::{PieceError, pieces};
use crate::config::command::{StdinError, check_command};
use crate::config::program::{expand_tilde, resolve_program};
pub use crate::config::reload::Reloader;
use crate::config::trust::check_file;

/// Default and ceiling for an action's timeout. Not configurable here: each action sets its
/// own `timeout` in actions.toml.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Not configurable: the ceiling is part of the actions.toml contract (docs/actions.md).
pub const MAX_TIMEOUT: Duration = Duration::from_secs(crate::numbers::timeout_secs!());
/// The refusal of a `timeout` that does not parse or is over [`MAX_TIMEOUT`].
const BAD_TIMEOUT: &str =
    concat!("timeout must be like 10s or 500ms, at most ", crate::numbers::timeout_secs!(), "s");
/// A config larger than this is not hand-written; refuse instead of parsing it.
/// Not configurable: a size bound read before parsing, against a file that is not a config.
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// Limits on `[action.vars]`: names `[a-z0-9_]{1,32}`, values are plain strings.
/// Not configurable: part of the payload v1 contract, and they bound the argv a var fills.
pub const MAX_VARS: usize = crate::numbers::max_vars!();
pub const MAX_VAR_NAME: usize = crate::numbers::max_vars!();
pub const MAX_VAR_BYTES: usize = crate::numbers::var_kib!() * 1024;
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

/// One validated action.
///
/// `argv` is a template: element 0 is a resolved, literal program and
/// every other element is expanded on its own, so a value is always exactly one argument and
/// never reaches a shell. A `shell` action's argv is `/bin/sh -c <script>`, passed verbatim:
/// its braces are shell text (`${HOME}`, awk's `{print}`), never placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub label: String,
    pub kinds: Vec<Kind>,
    pub argv: Vec<String>,
    pub shell: bool,
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
    /// `false`: checked like any other, but not offered and its program need not exist yet.
    #[serde(default = "yes")]
    enabled: bool,
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

const fn yes() -> bool {
    true
}

impl Config {
    /// The starter file `git terrace init` writes, parsed by the same rules as the
    /// user's own file.
    pub const STARTER: &'static str = include_str!("../defaults/actions.toml");

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
            return Err(ConfigError::Parse(format!("unsupported version {}", raw.version)));
        }
        let mut scripts = BTreeMap::new();
        for sc in raw.script {
            if !valid_id(&sc.id) {
                return Err(script_err(&sc.id, "id must match [a-z0-9-]{1,32}"));
            }
            if scripts.contains_key(&sc.id) {
                return Err(script_err(&sc.id, "duplicate id"));
            }
            check_command(&sc.command, sc.shell, None)
                .map_err(|why| script_err(&sc.id, why.to_string()))?;
            scripts.insert(sc.id.clone(), sc);
        }
        let mut seen = HashSet::new();
        let mut actions = Vec::with_capacity(raw.action.len());
        for a in raw.action {
            let enabled = a.enabled;
            let action = validate(a, &scripts, home)?;
            if !seen.insert(action.id.clone()) {
                return Err(invalid(&action.id, "duplicate id"));
            }
            if enabled {
                if let Some(k) = action.key {
                    if let Some(who) = RESERVED_KEYS.iter().find(|(b, _)| *b == k) {
                        return Err(invalid(
                            &action.id,
                            format!("key {k:?} is already used by {}", who.1),
                        ));
                    }
                    if let Some(other) = actions.iter().find(|o: &&Action| o.key == Some(k)) {
                        return Err(invalid(
                            &action.id,
                            format!("key {k:?} is already used by action {}", other.id),
                        ));
                    }
                }
                actions.push(action);
            }
        }
        let mut seen_sends = HashSet::new();
        let mut sends = Vec::with_capacity(raw.send.len());
        for raw_send in raw.send {
            let id = raw_send.id().to_owned();
            let enabled = raw_send.enabled;
            let target =
                crate::send::compile(raw_send).map_err(|why| send_err(&id, why.to_string()))?;
            if !valid_id(&id) {
                return Err(send_err(&id, "id must match [a-z0-9-]{1,32}"));
            }
            if !seen_sends.insert(id.clone()) {
                return Err(send_err(&id, "duplicate id"));
            }
            if enabled {
                sends.push(target);
            }
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

/// The home folder, or the filesystem root when its variable is unset.
pub(crate) fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR_STR), PathBuf::from)
}

fn invalid(id: &str, why: impl Into<String>) -> ConfigError {
    let (id, why) = (id.to_string(), why.into());
    ConfigError::Invalid { id, why }
}

fn script_err(id: &str, why: impl Into<String>) -> ConfigError {
    let (id, why) = (id.to_string(), why.into());
    ConfigError::Script { id, why }
}

fn send_err(id: &str, why: impl Into<String>) -> ConfigError {
    let (id, why) = (id.to_string(), why.into());
    ConfigError::Send { id, why }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
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
            (sc.command.clone(), sc.shell, sc.cwd.clone(), sc.timeout.clone(), sc.stdin.clone())
        }
    };
    check_command(&command, shell, Some(&a.vars)).map_err(|why| invalid(&id, why.to_string()))?;
    let stdin = parse_stdin(a.stdin.or(base_stdin).as_deref())
        .map_err(|why| invalid(&id, why.to_string()))?;
    let argv = match (shell, command) {
        (false, RawCommand::Argv(v)) if !v.is_empty() => {
            let mut v = v;
            match resolve_program(&v[0], home) {
                Some(p) => v[0] = p,
                // A disabled action may name a program not installed yet.
                None if !a.enabled => {}
                None => {
                    return Err(invalid(&id, format!("{} is not an executable file", v[0])));
                }
            }
            v
        }
        (false, _) => {
            return Err(invalid(&id, "command must be a non-empty array when shell = false"));
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
                return Err(invalid(&id, "cwd must be repo, home or an existing absolute dir"));
            }
            Cwd::Path(p)
        }
    };
    let timeout = match a.timeout.or(base_timeout).as_deref() {
        None => DEFAULT_TIMEOUT,
        Some(t) => parse_duration(t)
            .filter(|d| !d.is_zero() && *d <= MAX_TIMEOUT)
            .ok_or_else(|| invalid(&id, BAD_TIMEOUT))?,
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
        shell,
        stdin,
        cwd,
        timeout,
        output: a.output.unwrap_or(Output::Toast),
        key,
        env: a.env,
        vars,
    })
}

/// Keys an action may not take: the action menu's own, and every single key the page binds.
///
/// The page keys come from `nav.js`. An action key is both its menu key and its page chord ("a"
/// then the key), and a page key would fire its page command too. A web test pins this table to
/// `nav.js`.
pub const RESERVED_KEYS: [(char, &str); 9] = [
    ('p', "the menu's Copy permalink"),
    ('m', "the menu's Copy as markdown"),
    ('s', "the menu's Send to\u{2026}"),
    ('g', "the page's go-to chord"),
    ('t', "the page's Go to file"),
    ('y', "the page's Permalink"),
    ('b', "the page's Branches and tags"),
    ('j', "the page's next row"),
    ('k', "the page's previous row"),
];

fn parse_key(key: Option<&str>) -> Result<Option<char>, &'static str> {
    let Some(k) = key else { return Ok(None) };
    let mut c = k.chars();
    match (c.next(), c.next()) {
        (Some(ch), None) if ch.is_ascii_alphanumeric() => Ok(Some(ch)),
        _ => Err("key must be one letter or digit"),
    }
}

fn parse_stdin(value: Option<&str>) -> Result<Stdin, StdinError> {
    match value {
        None | Some("json") => Ok(Stdin::Json),
        Some("markdown") => Ok(Stdin::Markdown),
        Some("none") => Ok(Stdin::None),
        Some(other) => Err(StdinError(other.to_owned())),
    }
}

fn validate_vars(
    id: &str,
    vars: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ConfigError> {
    check_vars(&vars).map_err(|why| invalid(id, why))?;
    Ok(vars)
}

/// The rules `[action.vars]` is loaded under, so the settings form refuses what the loader
/// would: the reason, or `Ok` when `vars` loads.
pub fn check_vars(vars: &BTreeMap<String, String>) -> Result<(), String> {
    if vars.len() > MAX_VARS {
        return Err(format!("at most {MAX_VARS} vars"));
    }
    for (name, value) in vars {
        let lower = name.to_ascii_lowercase();
        let fixable = lower.len() <= MAX_VAR_NAME
            && lower.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if *name != lower && fixable {
            return Err(format!("vars.{name}: use lowercase: {lower}"));
        }
        if name.is_empty()
            || name.len() > MAX_VAR_NAME
            || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(format!("vars.{name}: name must match [a-z0-9_]{{1,32}}"));
        }
        if value.len() > MAX_VAR_BYTES {
            return Err(format!("vars.{name}: value over {} KiB", MAX_VAR_BYTES / 1024));
        }
        if value.contains('\0') {
            return Err(format!("vars.{name}: value contains NUL"));
        }
    }
    Ok(())
}

fn parse_duration(t: &str) -> Option<Duration> {
    if let Some(ms) = t.strip_suffix("ms") {
        return ms.parse().ok().map(Duration::from_millis);
    }
    t.strip_suffix('s')?.parse().ok().map(Duration::from_secs)
}

// The fixtures use /bin/sh, chmod modes and symlinks, so they run on unix only.
#[cfg(test)]
#[cfg(unix)]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "the tests spell action placeholders like {title}, which are not format strings"
)]
mod tests;
