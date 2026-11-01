//! Every setting, declared once. A doc comment is the description in `docs/configuration.md`.
//! A knob is here only if a user would turn it; everything else stays a constant.

use std::collections::BTreeMap;

use crate::value::Show;

/// One setting, for the loader, `git terrace config` and the docs page.
pub struct Key {
    pub section: &'static str,
    pub name: &'static str,
    /// `section.name`.
    pub id: &'static str,
    pub ty: &'static str,
    pub default: String,
    pub doc: &'static str,
}

macro_rules! settings {
    ($( $field:ident : $sec:ident { $(
        $(#[doc = $doc:literal])* $name:ident : $ty:ty = $default:expr ,
    )* } )*) => {
        $(
            #[derive(Debug, Clone, PartialEq)]
            pub struct $sec { $( $(#[doc = $doc])* pub $name: $ty, )* }
            impl Default for $sec {
                fn default() -> Self { Self { $( $name: $default, )* } }
            }
        )*

        /// The effective configuration.
        #[derive(Debug, Clone, Default, PartialEq)]
        pub struct Config {
            $( pub $field: $sec, )*
            /// Per-repository overrides, by repository name.
            pub repo: BTreeMap<String, RepoOverride>,
        }

        /// Every setting in declaration order.
        pub fn keys() -> Vec<Key> {
            let d = Config::default();
            vec![ $( $( Key {
                section: stringify!($field),
                name: stringify!($name),
                id: concat!(stringify!($field), ".", stringify!($name)),
                ty: <$ty as Show>::TYPE,
                default: Show::show(&d.$field.$name),
                doc: concat!($($doc, "\n"),*).trim(),
            }, )* )* ]
        }

        impl Config {
            /// The value of `k` as a TOML literal.
            pub fn show(&self, k: &Key) -> String {
                match k.id {
                    $( $( concat!(stringify!($field), ".", stringify!($name)) =>
                        Show::show(&self.$field.$name), )* )*
                    _ => String::new(),
                }
            }

            pub(crate) fn from_table(mut t: toml::Table) -> Result<Self, String> {
                let mut c = Self::default();
                $(
                    if let Some(v) = t.remove(stringify!($field)) {
                        if let Some(sub) = v.as_table() {
                            $( if let Some(x) = sub.get(stringify!($name)) {
                                c.$field.$name = <$ty as Show>::from_toml(x).ok_or_else(|| format!(
                                    "{}.{}: expected {}",
                                    stringify!($field), stringify!($name), <$ty as Show>::TYPE))?;
                            } )*
                        }
                    }
                )*
                if let Some(r) = t.remove("repo") {
                    let tbl = r.as_table().ok_or("repo: expected a table of repositories")?;
                    for (name, o) in tbl {
                        let ex = o.get("exclude").map_or(Ok(Vec::new()), |x| {
                            <Vec<String> as Show>::from_toml(x)
                                .ok_or_else(|| format!("repo.{name}.exclude: expected list of strings"))
                        })?;
                        c.repo.insert(name.clone(), RepoOverride { exclude: ex });
                    }
                }
                Ok(c)
            }
        }
    };
}

/// Overrides for one repository, matched by its folder name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RepoOverride {
    /// Paths (globs) left out of every insight for this repository, added to `insights.exclude`.
    pub exclude: Vec<String>,
}

settings! {
    server: Server {
        /// Address to listen on. Keep it local: pages show source code and there is no login. Needs a
        /// restart.
        addr: String = "127.0.0.1:4242".to_owned(),
        /// Maximum open live-update streams per repository (one per browser tab).
        max_streams: usize = 32,
    }
    watch: Watch {
        /// Notice new commits and refs while running. Off means a restart or a reload is needed.
        enabled: bool = true,
        /// How often to check each served repository for changes, in milliseconds. A check is a
        /// few `stat` calls. Minimum 250.
        interval_ms: u64 = 1000,
    }
    index: Index {
        /// Build the commit index as soon as a repository is served: at startup for one
        /// repository, at its first visit when several share the server. Off waits for the first
        /// page that needs it (Insights).
        build_on_start: bool = true,
        /// Threads used to parse the index. 0 means one per core, capped at 8. Needs a restart.
        threads: usize = 0,
    }
    resources: Resources {
        /// Memory for the blame and highlighted-code caches together, in MB, shared half and
        /// half. The highlighted-code cache is per process, the blame cache per repository.
        cache_mb: usize = 64,
        /// Most page-building git jobs running at once across all repositories. 0 means one per
        /// core.
        blocking_jobs: usize = 0,
    }
    limits: Limits {
        /// Largest file shown as text, in KiB. Larger files are offered as raw downloads.
        text_kb: u64 = 2048,
        /// Largest file served raw, in MB.
        raw_mb: u64 = 256,
        /// Most matches a code search returns.
        search_matches: usize = 500,
        /// Lines of one file's diff shown before it folds.
        diff_fold_lines: usize = 400,
    }
    insights: Insights {
        /// Paths (globs) left out of every insight.
        exclude: Vec<String> = Vec::new(),
        /// Count commits by bots.
        bots: bool = false,
        /// Count lock files and minified output.
        generated: bool = false,
        /// Period the Pulse, Hotspots, Ownership, Patterns, Code frequency and Contributors
        /// pages use when the URL names none: 1w, 1m, 3m, 1y or all. All leaves each page its own
        /// default (Pulse opens on a month).
        period: String = "all".to_owned(),
    }
    display: Display {
        /// Color theme before the visitor picks one: system, light or dark.
        theme: String = "system".to_owned(),
        /// How dates read: relative or absolute.
        dates: String = "relative".to_owned(),
    }
    multi: Multi {
        /// How many folder levels to search below a given folder for repositories. Needs a restart.
        depth: usize = 2,
        /// Folder names never searched, besides hidden ones. Needs a restart.
        skip_dirs: Vec<String> = vec!["node_modules".to_owned(), "target".to_owned(), ".wt".to_owned()],
    }
}

impl Config {
    /// Rejects values a running server cannot honour. Errors name the key.
    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        let bad = |k: &str, why: &str| Err(crate::ConfigError(format!("{k}: {why}")));
        if self.server.addr.parse::<std::net::SocketAddr>().is_err() {
            return bad(
                "server.addr",
                "expected host:port, for example 127.0.0.1:4242",
            );
        }
        if self.server.max_streams == 0 {
            return bad("server.max_streams", "must be at least 1");
        }
        if self.watch.interval_ms < 250 {
            return bad("watch.interval_ms", "must be at least 250");
        }
        if self.index.threads > 64 {
            return bad("index.threads", "must be at most 64");
        }
        if self.resources.cache_mb == 0 || self.resources.cache_mb > 16_384 {
            return bad("resources.cache_mb", "must be between 1 and 16384");
        }
        if self.limits.text_kb == 0 || self.limits.raw_mb == 0 {
            return bad("limits", "text_kb and raw_mb must be at least 1");
        }
        if !["1w", "1m", "3m", "1y", "all"].contains(&self.insights.period.as_str()) {
            return bad("insights.period", "expected 1w, 1m, 3m, 1y or all");
        }
        if !["system", "light", "dark"].contains(&self.display.theme.as_str()) {
            return bad("display.theme", "expected system, light or dark");
        }
        if !["relative", "absolute"].contains(&self.display.dates.as_str()) {
            return bad("display.dates", "expected relative or absolute");
        }
        if self.multi.depth == 0 || self.multi.depth > 6 {
            return bad("multi.depth", "must be between 1 and 6");
        }
        Ok(())
    }
}
