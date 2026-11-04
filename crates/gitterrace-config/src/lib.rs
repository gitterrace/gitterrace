//! The one user configuration: `config.toml` in [`config_dir`]. Never read from a served
//! repository. Precedence is flag over file over default; [`Loaded::describe`] shows which won.
//!
//! The settings are declared once in the `settings!` call in `schema.rs`. Its doc comments are
//! the descriptions in `docs/configuration.md`, so the file, the loader and the page cannot drift.

mod budget;
mod features;
#[cfg(test)]
mod limits_tests;
pub mod naming;
mod schema;
mod value;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub use budget::{Cache, PARTS, SHARES};
pub use features::{Feature, INSIGHT_PAGES};
pub use schema::{Api, BOUNDS, Config, DEFAULT_ADDR, Key, Reload, RepoOverride, keys};
pub use value::Show;

/// Where a value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    File,
    Flag,
}

impl Source {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::File => "file",
            Self::Flag => "flag",
        }
    }
}

/// Why the config could not be used. Names the key so the user can fix it.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Where the config directory is.
///
/// On Windows `%APPDATA%\gitterrace`. Else `$XDG_CONFIG_HOME/gitterrace` when that is absolute,
/// else `$HOME/.config/gitterrace` (also on macOS). `override_dir` is the `--config` flag. No other environment variable moves it.
pub fn config_dir(override_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(d) = override_dir {
        return Some(d.to_path_buf());
    }
    #[cfg(windows)]
    {
        // The roaming profile folder (`%APPDATA%`), private to the user by default.
        if let Some(a) = std::env::var_os("APPDATA").map(PathBuf::from) {
            return Some(a.join("gitterrace"));
        }
    }
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    if let Some(x) = xdg.filter(|p| p.is_absolute()) {
        return Some(x.join("gitterrace"));
    }
    home_var().map(|h| PathBuf::from(h).join(".config/gitterrace"))
}

/// The home folder variable: `USERPROFILE` on Windows, `HOME` elsewhere.
pub fn home_var() -> Option<std::ffi::OsString> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
}

/// A loaded config plus where each key came from and what was ignored.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    pub sources: BTreeMap<&'static str, Source>,
    /// Unknown keys and similar; shown on stderr, never fatal.
    pub warnings: Vec<String>,
}

impl Loaded {
    /// Records that a command-line flag set `key` (`section.name`).
    pub fn flagged(&mut self, key: &'static str) {
        self.sources.insert(key, Source::Flag);
    }

    /// Fails with the key name when a value is out of range.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.config.validate()
    }

    /// One line per key: `key = value  (source)`.
    pub fn describe(&self) -> String {
        let mut out = String::new();
        for k in &keys() {
            let name = format!("{}.{}", k.section, k.name);
            let src = self
                .sources
                .get(name.as_str())
                .copied()
                .unwrap_or(Source::Default);
            let _ = writeln!(out, "{name} = {}  ({})", self.config.show(k), src.label());
        }
        for (repo, o) in &self.config.repo {
            let _ = writeln!(out, "repo.{repo}.exclude = {:?}  (file)", o.exclude);
            for f in &o.features_off {
                let _ = writeln!(out, "repo.{repo}.features.{} = false  (file)", f.key());
            }
        }
        out
    }
}

/// Reads `config.toml` from `dir`. A missing file is the defaults; a malformed one is an error.
pub fn load(dir: Option<&Path>) -> Result<Loaded, ConfigError> {
    let text = match dir.map(|d| d.join("config.toml")) {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(t) => Some((p, t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(ConfigError(format!("{}: {e}", p.display()))),
        },
        None => None,
    };
    match text {
        Some((p, t)) => parse(&t).map_err(|e| ConfigError(format!("{}: {}", p.display(), e.0))),
        None => Ok(parse("").unwrap_or_else(|_| unreachable_defaults())),
    }
}

/// A documented `config.toml` template: every key commented out at its default with its
/// description above it, grouped by section. It changes nothing until a line is uncommented.
pub fn template() -> String {
    let mut out = String::from(
        "# gitterrace config.toml. Every line is commented out at its default: uncomment to change.\n",
    );
    let mut section = "";
    for k in schema::keys() {
        if k.section != section {
            section = k.section;
            out.push_str(&["\n[", section, "]\n"].concat());
        }
        out.push('\n');
        for line in k.doc.lines() {
            out.push_str(&["# ", line.trim(), "\n"].concat());
        }
        out.push_str(&["# ", k.name, " = ", &k.default, "\n"].concat());
    }
    out
}

fn unreachable_defaults() -> Loaded {
    Loaded {
        config: Config::default(),
        sources: BTreeMap::new(),
        warnings: Vec::new(),
    }
}

/// Parses config text. Unknown keys become warnings.
pub fn parse(text: &str) -> Result<Loaded, ConfigError> {
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| ConfigError(e.to_string()))?;
    let mut warnings = Vec::new();
    let mut sources = BTreeMap::new();
    for (section, v) in &table {
        let Some(sub) = v.as_table() else {
            warnings.push(format!("unknown key `{section}` ignored"));
            continue;
        };
        if section == "repo" {
            continue;
        }
        for name in sub.keys() {
            match keys()
                .iter()
                .find(|k| k.section == section && k.name == name)
            {
                Some(k) => {
                    sources.insert(k.id, Source::File);
                }
                None => warnings.push(format!("unknown key `{section}.{name}` ignored")),
            }
        }
    }
    let config = Config::from_table(table).map_err(ConfigError)?;
    config.validate()?;
    Ok(Loaded {
        config,
        sources,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::{Config, Reload, Source, keys, load, parse, template};

    /// The serving cache budget is the one the memory guard assumes (32 MB plus margin): a
    /// larger default breaks the guard on every machine, not just a small one.
    #[test]
    fn cache_budget_default_matches_the_memory_design() {
        let design = include_str!("../../../docs/design/streaming-insights.md");
        assert!(design.contains("(default 32 MB"), "design moved its budget");
        assert_eq!(Config::default().resources.cache_mb, 32);
    }

    /// The docs page and the reload read the same field: a key documented as needing a
    /// restart is one the reload holds back, and the other way round.
    #[test]
    fn restart_keys_match_their_docs_and_are_held_back() {
        let mut changed = parse(
            "[server]\naddr = \"127.0.0.1:9\"\nis_public = true\nhosts = [\"h.example\"]\nlive_streams = 3\n\
             public_timeout_secs = 7\n\
             [index]\nthreads = 3\n[multi]\ndepth = 4\nskip_dirs = [\"x\"]\n\
             [limits]\nunindexed_commits = 7\naction_output_kb = 7\nregistry_entries = 7\n",
        )
        .expect("parse")
        .config;
        let old = Config::default();
        let restart: Vec<&str> = keys()
            .iter()
            .filter(|k| k.reload == Reload::Restart)
            .map(|k| k.id)
            .collect();
        for k in keys() {
            let says = k
                .doc
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains("Needs a restart");
            assert_eq!(
                says,
                k.reload == Reload::Restart,
                "{}: doc and table disagree",
                k.id
            );
        }
        assert_eq!(old.restart_changes(&changed), restart);
        changed.keep_restart(&old);
        assert!(old.restart_changes(&changed).is_empty());
        assert_eq!(
            changed.server.live_streams, 3,
            "live keys are not held back"
        );
    }

    #[test]
    fn template_loads_to_the_defaults_without_warnings() {
        let l = parse(&template()).expect("parse");
        assert_eq!(l.config, crate::Config::default());
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        assert!(l.sources.is_empty());
    }

    #[test]
    fn defaults_keep_todays_behaviour() {
        let l = parse("").expect("parse");
        assert_eq!(l.config.watch.interval_ms, 1000);
        assert_eq!(l.config.server.addr, crate::DEFAULT_ADDR);
        assert!(l.config.watch.enabled);
        assert!(l.sources.is_empty());
    }

    #[test]
    fn file_sets_values_and_source() {
        let l = parse("[watch]\ninterval_ms = 500\n[repo.example]\nexclude = [\"vendor/**\"]\n")
            .expect("parse");
        assert_eq!(l.config.watch.interval_ms, 500);
        assert_eq!(l.sources["watch.interval_ms"], Source::File);
        assert_eq!(l.config.repo["example"].exclude, ["vendor/**"]);
        assert!(l.describe().contains("watch.interval_ms = 500  (file)"));
    }

    #[test]
    fn unknown_keys_warn_and_invalid_values_name_the_key() {
        let l = parse("[watch]\nbogus = 1\n[nope]\nx = 1\n").expect("parse");
        assert_eq!(l.warnings.len(), 2);
        let e = parse("[watch]\ninterval_ms = 10\n").expect_err("too fast");
        assert!(e.0.contains("watch.interval_ms"), "{e}");
        let e = parse("[server]\naddr = \"nope\"\n").expect_err("bad addr");
        assert!(e.0.contains("server.addr"), "{e}");
        let e = parse("[watch]\nenabled = 3\n").expect_err("type");
        assert!(e.0.contains("watch.enabled"), "{e}");
    }

    /// A sidebar section name is checked, never silently dropped, and the error lists the
    /// valid ones. An empty list is valid: it hides the sidebar.
    #[test]
    fn display_sidebar_names_are_checked() {
        let e = parse("[display]\nsidebar = [\"now\", \"churm\"]\n").expect_err("typo");
        assert!(
            e.0.contains("display.sidebar") && e.0.contains("churm"),
            "{e}"
        );
        assert!(e.0.contains("loose_ends"), "names the valid ones: {e}");
        let l = parse("[display]\nsidebar = []\n").expect("empty is valid");
        assert!(l.config.display.sidebar.is_empty());
    }

    /// Every address another machine can reach needs `public`; loopback never does.
    #[test]
    fn a_reachable_address_needs_public() {
        for addr in [
            "0.0.0.0:4250",
            "[::]:4250",
            "192.168.1.5:4250",
            "10.0.0.1:80",
        ] {
            let e = parse(&format!("[server]\naddr = \"{addr}\"\n")).expect_err(addr);
            assert!(e.0.contains("server.is_public"), "{addr}: {e}");
            parse(&format!("[server]\naddr = \"{addr}\"\nis_public = true\n")).expect(addr);
        }
        for addr in ["127.0.0.1:4250", "[::1]:4250", "127.0.0.2:4250"] {
            parse(&format!("[server]\naddr = \"{addr}\"\n")).expect(addr);
        }
    }

    #[test]
    fn hosts_are_plain_names_and_need_public() {
        let e = parse("[server]\nhosts = [\"git.example.com\"]\n").expect_err("no public");
        assert!(e.0.contains("server.hosts"), "{e}");
        for h in ["git.example.com", "localhost", "203.0.113.4", "a-b.example"] {
            parse(&format!("[server]\nis_public = true\nhosts = [\"{h}\"]\n")).expect(h);
        }
        for h in [
            "*.example.com",
            "https://git.example.com",
            "git.example.com:443",
            "",
            "git..example",
            "-x.example",
            "git.example.com/",
        ] {
            let e =
                parse(&format!("[server]\nis_public = true\nhosts = [\"{h}\"]\n")).expect_err(h);
            assert!(e.0.contains("server.hosts"), "{h}: {e}");
        }
    }

    #[test]
    fn missing_file_is_defaults_and_flag_wins() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut l = load(Some(tmp.path())).expect("load");
        l.config.watch.enabled = false;
        l.flagged("watch.enabled");
        assert!(l.describe().contains("watch.enabled = false  (flag)"));
    }
}
