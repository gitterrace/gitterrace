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
    /// Whether a running server takes a new value on reload.
    pub reload: Reload,
}

/// How an edit to one setting reaches a running server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reload {
    /// Applied on the next reload of `config.toml`.
    Live,
    /// Read once at startup: a reload keeps the running value and says a restart is needed.
    Restart,
}

macro_rules! settings {
    ($( $field:ident : $sec:ident { $(
        $(#[doc = $doc:literal])* $name:ident : $ty:ty = $default:expr => $reload:ident ,
    )* } )*) => {
        $(
            #[derive(Debug, Clone, PartialEq, Eq)]
            pub struct $sec { $( $(#[doc = $doc])* pub $name: $ty, )* }
            impl Default for $sec {
                fn default() -> Self { Self { $( $name: $default, )* } }
            }
        )*

        /// The effective configuration.
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
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
                reload: Reload::$reload,
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

            /// Ids of the [`Reload::Restart`] settings whose value differs in `new`.
            pub fn restart_changes(&self, new: &Self) -> Vec<&'static str> {
                let mut out = Vec::new();
                $( $( if Reload::$reload == Reload::Restart && self.$field.$name != new.$field.$name {
                    out.push(concat!(stringify!($field), ".", stringify!($name)));
                } )* )*
                out
            }

            /// Copies every [`Reload::Restart`] setting from `running`, so the result holds only
            /// values a running server can put in force.
            pub fn keep_restart(&mut self, running: &Self) {
                $( $( if Reload::$reload == Reload::Restart {
                    self.$field.$name.clone_from(&running.$field.$name);
                } )* )*
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoOverride {
    /// Paths (globs) left out of every insight for this repository, added to `insights.exclude`.
    pub exclude: Vec<String>,
}

/// The address the server listens on when nothing overrides it: the one place it is written.
pub const DEFAULT_ADDR: &str = "127.0.0.1:4242";

settings! {
    server: Server {
        /// Address to listen on. Keep it local: pages show source code and there is no login. Needs a
        /// restart.
        addr: String = DEFAULT_ADDR.to_owned() => Restart,
        /// Maximum open live-update streams per repository (one per browser tab).
        max_streams: usize = 32 => Live,
        /// Serve read-only to anyone who can reach the address: committed history only, no
        /// working tree, stashes, live updates, actions or additions. Required for any address
        /// that is not loopback. Needs a restart.
        public: bool = false => Restart,
        /// Host names a reverse proxy forwards with, exact and without port, for example
        /// `["git.example.com"]`. Needs `public`. Needs a restart.
        hosts: Vec<String> = Vec::new() => Restart,
    }
    watch: Watch {
        /// Notice new commits and refs while running. Off means a restart or a reload is needed.
        enabled: bool = true => Live,
        /// How often to check each served repository for changes, in milliseconds. A check is a
        /// few `stat` calls. Minimum 250.
        interval_ms: u64 = 1000 => Live,
    }
    index: Index {
        /// Build the commit index as soon as a repository is served: at startup for one
        /// repository, at its first visit when several share the server. Off waits for the first
        /// page that needs it (Insights).
        build_on_start: bool = true => Live,
        /// Threads that parse the index. 0 means one per core, capped at 8. Needs a restart.
        threads: usize = 0 => Restart,
    }
    resources: Resources {
        /// Memory for the serving caches of one repository together, in MB: a quarter each for
        /// blame and highlighted code, an eighth each for rendered pages, file lists, last
        /// commits and path logs. An entry over an eighth of its cache is not kept.
        cache_mb: usize = 32 => Live,
        /// Most page-building git jobs running at once across all repositories. 0 means one per
        /// core.
        blocking_jobs: usize = 0 => Live,
    }
    limits: Limits {
        /// Largest file shown as text, in KiB. Larger files are offered as raw downloads.
        text_kb: u64 = 2048 => Live,
        /// Largest file served raw, in MB.
        raw_mb: u64 = 256 => Live,
        /// Most matches a code search returns.
        search_matches: usize = 500 => Live,
        /// Lines of one file's diff shown before it folds.
        diff_fold_lines: usize = 400 => Live,
    }
    insights: Insights {
        /// Paths (globs) left out of every insight.
        exclude: Vec<String> = Vec::new() => Live,
        /// Count commits by bots.
        bots: bool = false => Live,
        /// Count lock files and minified output.
        generated: bool = false => Live,
        /// Period the insights pages use when the URL names none: 1w, 1m, 3m, 1y or all. The
        /// API and MCP default to all. Their `period` parameter picks one.
        period: String = "all".to_owned() => Live,
    }
    display: Display {
        /// Color theme before the visitor picks one: system, light or dark.
        theme: String = "system".to_owned() => Live,
        /// How dates read: relative or absolute.
        dates: String = "relative".to_owned() => Live,
    }
    multi: Multi {
        /// How many folder levels to search below a given folder for repositories. Needs a restart.
        depth: usize = 2 => Restart,
        /// Folder names never searched, besides hidden ones. Needs a restart.
        skip_dirs: Vec<String> = vec!["node_modules".to_owned(), "target".to_owned(), ".wt".to_owned()] => Restart,
    }
    links: Links {
        /// Form of the permalink Copy permalink, Send to, action payloads and the MCP hand out:
        /// gitterrace (`gitterrace://<id>/<sha>/<path>`, opened by `git terrace open` without a
        /// running server) or go (`http://<addr>/go/...`, needs this server running).
        style: String = "gitterrace".to_owned() => Live,
        /// Folders `git terrace open` searches, `multi.depth` levels deep, for a clone a link
        /// names, besides every repository served before.
        roots: Vec<String> = Vec::new() => Live,
    }
}

impl Config {
    /// Rejects values a running server cannot honour. Errors name the key.
    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        let bad = |k: &str, why: &str| Err(crate::ConfigError(format!("{k}: {why}")));
        if self.server.addr.parse::<std::net::SocketAddr>().is_err() {
            return bad(
                "server.addr",
                &format!("expected host:port, for example {DEFAULT_ADDR}"),
            );
        }
        if let Some(why) = self.public_problem() {
            return bad(why.0, &why.1);
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
        if !["gitterrace", "go"].contains(&self.links.style.as_str()) {
            return bad("links.style", "expected gitterrace or go");
        }
        if self.multi.depth == 0 || self.multi.depth > 6 {
            return bad("multi.depth", "must be between 1 and 6");
        }
        Ok(())
    }

    /// Why the online settings cannot be honored: a non-loopback address without
    /// `server.public` (fails closed, so a typo in `addr` never exposes the working tree), or a
    /// `server.hosts` entry that is not one plain host name.
    fn public_problem(&self) -> Option<(&'static str, String)> {
        let s = &self.server;
        let addr = s.addr.parse::<std::net::SocketAddr>().ok()?;
        if !addr.ip().is_loopback() && !s.public {
            return Some((
                "server.addr",
                format!(
                    "{addr} is reachable from other machines. Set server.public = true (or pass --public) to serve read-only there"
                ),
            ));
        }
        if !s.hosts.is_empty() && !s.public {
            return Some(("server.hosts", "needs server.public = true".to_owned()));
        }
        s.hosts.iter().find(|h| !plain_host(h)).map(|h| {
            (
                "server.hosts",
                format!("{h:?}: expected a host name without scheme, port or wildcard"),
            )
        })
    }
}

/// A DNS name or IP literal, nothing else: no `*`, scheme, port, path or empty label.
fn plain_host(h: &str) -> bool {
    if h.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    !h.is_empty()
        && h.len() <= 253
        && h.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
