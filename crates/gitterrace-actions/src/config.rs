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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    Toast,
    Ignore,
}

/// One validated action. `argv` is exactly what is spawned; nothing from a request is ever
/// substituted into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub label: String,
    pub kinds: Vec<Kind>,
    pub argv: Vec<String>,
    pub cwd: Cwd,
    pub timeout: Duration,
    pub output: Output,
    pub key: Option<char>,
    /// Extra variable names passed through from gitterrace's own environment.
    pub env: Vec<String>,
    /// Fixed values from `[action.vars]`, delivered as `GITTERRACE_VAR_<NAME>` and
    /// `payload.vars`. Only `actions.toml` sets them; never templated into `argv`.
    pub vars: BTreeMap<String, String>,
}

/// The loaded set, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub actions: Vec<Action>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    version: u32,
    #[serde(default)]
    script: Vec<RawScript>,
    #[serde(default)]
    action: Vec<RawAction>,
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
        Ok(Self { actions })
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
    let (command, shell, base_cwd, base_timeout) = match (a.script, a.command) {
        (Some(_), Some(_)) => return Err(invalid(&id, "set either script or command, not both")),
        (None, None) => return Err(invalid(&id, "command or script is required")),
        (None, Some(c)) => (c, a.shell.unwrap_or(false), None, None),
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
            )
        }
    };
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
    let key = match a.key.as_deref() {
        None => None,
        Some(k) => {
            let mut c = k.chars();
            match (c.next(), c.next()) {
                (Some(ch), None) if ch.is_ascii_alphanumeric() => Some(ch),
                _ => return Err(invalid(&id, "key must be one letter or digit")),
            }
        }
    };
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
        cwd,
        timeout,
        output: a.output.unwrap_or(Output::Toast),
        key,
        env: a.env,
        vars,
    })
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
}
