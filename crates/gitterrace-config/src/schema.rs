//! Every setting, declared once. A doc comment is the description in `docs/configuration.md`.
//! A knob is here only if a user would turn it; everything else stays a constant.

use std::collections::BTreeMap;

use crate::features::{Feature, INSIGHT_PAGES};
use crate::value::Show;

/// The overview sidebar's sections, in their default order.
pub const SIDEBAR: [&str; 5] = ["now", "recent", "churn", "loose_ends", "languages"];

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
    /// Whether a larger value makes one request cost more (time or memory). A public server
    /// logs a warning at start for each such key set above its default.
    pub cost: bool,
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
    ($( $(#[$attr:meta])* $field:ident : $sec:ident { $(
        $(#[doc = $doc:literal])* $name:ident : $ty:ty = $default:expr => $reload:ident $(+ $cost:ident)? ,
    )* } )*) => {
        $(
            #[derive(Debug, Clone, PartialEq, Eq)]
            $(#[$attr])*
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
                cost: { let c: &[&str] = &[$(stringify!($cost)),*]; !c.is_empty() },
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

            /// Ids of the cost-tagged settings (see [`Key::cost`]) set above their default.
            pub fn costs_above_default(&self) -> Vec<&'static str> {
                let d = Self::default();
                let mut out = Vec::new();
                $( $( $(
                    let num = |v: String| v.parse::<u64>().unwrap_or(0);
                    if stringify!($cost) == "cost" && num(Show::show(&self.$field.$name)) > num(Show::show(&d.$field.$name)) {
                        out.push(concat!(stringify!($field), ".", stringify!($name)));
                    }
                )? )* )*
                out
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
                        let off = repo_features(name, o.get("features"))?;
                        c.repo.insert(name.clone(), RepoOverride { exclude: ex, features_off: off });
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
    /// Features `[repo.NAME.features]` turns off for this repository. An override only turns
    /// off: `true` is refused, so it never widens what the server-wide `[features]` serves.
    pub features_off: Vec<Feature>,
}

/// The features `[repo.NAME.features]` turns off. Unknown, server-wide or `true` keys are
/// errors naming the key.
fn repo_features(name: &str, t: Option<&toml::Value>) -> Result<Vec<Feature>, String> {
    let Some(t) = t else { return Ok(Vec::new()) };
    let t = t
        .as_table()
        .ok_or_else(|| format!("repo.{name}.features: expected a table"))?;
    let mut off = Vec::new();
    for (k, v) in t {
        let id = format!("repo.{name}.features.{k}");
        let f = Feature::from_key(k).ok_or_else(|| format!("{id}: unknown feature"))?;
        if f.is_global_only() {
            return Err(format!(
                "{id}: a server-wide feature, set it under [features]"
            ));
        }
        match v.as_bool() {
            Some(false) => off.push(f),
            Some(true) => {
                return Err(format!(
                    "{id}: an override only turns a feature off, set it under [features]"
                ));
            }
            None => return Err(format!("{id}: expected bool")),
        }
    }
    Ok(off)
}

/// Range of every bounded number: (id, least, most). The least is never 0, so a limit cannot
/// switch its page off; the most keeps one request within a machine.
pub const BOUNDS: &[(&str, u64, u64)] = &[
    ("server.public_timeout_secs", 1, 3600),
    ("limits.search_file_lines", 1, 100_000),
    ("limits.search_commits", 1, 100_000),
    ("limits.blob_lines", 1, 10_000_000),
    ("limits.highlight_kb", 1, 65_536),
    ("limits.log_pages", 1, 1_000_000),
    ("limits.preview_mb", 1, 4096),
    ("limits.worktree_files", 1, 100_000),
    ("limits.worktree_diff_kb", 1, 262_144),
    ("limits.unindexed_commits", 1, 200_000),
    ("limits.action_output_kb", 1, 65_536),
    ("limits.registry_entries", 1, 100_000),
    ("limits.export_pages", 1, 10_000_000),
    ("api.log_page_commits", 1, 100_000),
    ("api.log_skip_commits", 1, 100_000_000),
    ("api.blob_lines", 1, 10_000_000),
    ("api.blob_kb", 1, 1_048_576),
    ("api.blame_lines", 1, 10_000_000),
    ("api.diff_lines", 1, 10_000_000),
];

/// The address the server listens on when nothing overrides it: the one place it is written.
pub const DEFAULT_ADDR: &str = "127.0.0.1:4242";

settings! {
    server: Server {
        /// Address to listen on. Keep it local: pages show source code and there is no login. Needs a
        /// restart.
        addr: String = DEFAULT_ADDR.to_owned() => Restart,
        /// Maximum open live-update streams per repository (one per browser tab).
        live_streams: usize = 32 => Live,
        /// Serve read-only to anyone who can reach the address: committed history only, no
        /// working tree, stashes, live updates, actions or additions. Required for any address
        /// that is not loopback. Needs a restart.
        is_public: bool = false => Restart,
        /// Host names a reverse proxy forwards with, exact and without port, for example
        /// `["git.example.com"]`. Needs `public`. Needs a restart.
        hosts: Vec<String> = Vec::new() => Restart,
        /// Longest a request to a public server may take, in seconds. Past it the answer is 503
        /// and the work it started finishes in the background, filling the cache for a retry.
        /// Needs a restart.
        public_timeout_secs: u64 = 30 => Restart + cost,
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
        /// Memory for the serving caches of one repository together, in MB, in sixteenths: three
        /// each for blame and highlighted code, four for index blocks (insights pages), two each
        /// for rendered pages and file lists, one each for last commits and path logs. An entry
        /// over an eighth of its cache is not kept. An idle server keeps only the rendered pages.
        cache_mb: usize = 32 => Live,
        /// Most page-building git jobs running at once across all repositories. 0 means one per
        /// core.
        blocking_jobs: usize = 0 => Live,
    }
    limits: Limits {
        /// Largest file shown as text, in KiB. Larger files are offered as raw downloads.
        text_kb: u64 = 2048 => Live + cost,
        /// Largest file served raw, in MB. A larger one is refused.
        raw_mb: u64 = 256 => Live + cost,
        /// Most matches a code search returns. Past it the search stops and says it was cut off.
        search_matches: usize = 500 => Live + cost,
        /// Matching lines one file keeps in a code search. Past it the file says more matched.
        search_file_lines: usize = 100 => Live + cost,
        /// Most commits a commit search lists. Past it the search stops and says it was cut off.
        search_commits: usize = 200 => Live + cost,
        /// Lines of one file's diff shown before it folds. A folded file loads on demand.
        diff_fold_lines: usize = 400 => Live + cost,
        /// Lines a file page shows. Past it the page cuts the file and says so. `api.blob_lines`
        /// is the API's own cut.
        blob_lines: usize = 20_000 => Live + cost,
        /// Largest text highlighted, in KiB: a file, its blame, one file's diff or a Markdown
        /// code block. Larger text shows plain.
        highlight_kb: usize = 256 => Live + cost,
        /// History pages a log reaches: a later page number shows the last of them.
        log_pages: usize = 1000 => Live + cost,
        /// Largest image whose pixel size a file page reads, in MB. A larger one shows no
        /// pixel size.
        preview_mb: u64 = 16 => Live + cost,
        /// Changed files the Uncommitted view lists. Past it the rest are left out.
        worktree_files: usize = 2000 => Live,
        /// Largest working-tree file the Uncommitted view diffs, in KiB. A larger one is listed
        /// without hunks.
        worktree_diff_kb: usize = 1024 => Live,
        /// Commits a view of a revision outside the indexed history reads from git per request.
        /// Past it the view asks for a reindex instead. Needs a restart.
        unindexed_commits: usize = 2000 => Restart + cost,
        /// Output kept from each stream (stdout, stderr) of an action, in KiB. The rest is read
        /// and dropped. Needs a restart.
        action_output_kb: usize = 64 => Restart,
        /// Repositories remembered in `repos.json`, newest first. Past it the least recently
        /// served are forgotten with their names. Needs a restart.
        registry_entries: usize = 500 => Restart,
        /// Most pages `git terrace export` writes, folded diff fragments included. Past it the
        /// export stops and writes nothing. `export --export-pages` overrides it. Between 1 and
        /// 10000000.
        export_pages: usize = 20_000 => Live,
    }
    api: Api {
        /// Most commits one `log` answer lists, whatever `per_page` asks.
        log_page_commits: usize = 500 => Live + cost,
        /// Commits a `log` answer may skip to reach its page. A deeper page is refused.
        log_skip_commits: usize = 50_000 => Live + cost,
        /// Most lines one `blob` answer carries, whatever `limit` asks. The rest is cut and
        /// `truncated` says so. `limits.blob_lines` is the file page's own cut.
        blob_lines: usize = 5000 => Live + cost,
        /// Most bytes one `blob` answer carries, in KiB: one minified line can be megabytes.
        /// The rest is cut and `truncated` says so.
        blob_kb: usize = 256 => Live + cost,
        /// Most lines one `blame` answer carries. The rest is cut and `truncated` says so.
        blame_lines: usize = 5000 => Live + cost,
        /// Most diff lines one commit answer carries, whatever `max_lines` asks. The rest is
        /// cut and `truncated` says so.
        diff_lines: usize = 20_000 => Live + cost,
    }
    insights: Insights {
        /// Paths (globs) left out of every insight.
        exclude: Vec<String> = Vec::new() => Live,
        /// Count commits by bots.
        include_bots: bool = false => Live,
        /// Count lock files and minified output.
        include_generated: bool = false => Live,
        /// Period the insights pages use when the URL names none: 1w, 1m, 3m, 1y or all. The
        /// API and MCP default to all. Their `period` parameter picks one.
        period: String = "all".to_owned() => Live,
        /// Insights pages, in the order the tabs show them: `activity`, `contributors`,
        /// `hotspots`, `languages`, `ownership`, `types`, `frequency` and `patterns`. Leave one
        /// out to stop serving it. `features.insights_enabled` switches them all.
        pages: Vec<String> = INSIGHT_PAGES.iter().map(|&s| s.to_owned()).collect() => Live,
    }
    display: Display {
        /// Color theme before the visitor picks one: system, light or dark.
        theme: String = "system".to_owned() => Live,
        /// How dates read: relative or absolute.
        dates: String = "relative".to_owned() => Live,
        /// Overview sidebar sections, in the order shown: `now`, `recent`, `churn`,
        /// `loose_ends` and `languages`. Leave one out to hide it. An empty list hides the
        /// sidebar.
        sidebar: Vec<String> = SIDEBAR.iter().map(|&s| s.to_owned()).collect() => Live,
    }
    #[expect(
        clippy::struct_field_names,
        reason = "gitterrace_config::naming: every feature switch ends in _enabled"
    )]
    features: Features {
        /// Serve who last changed each line of a file (`/blame`) and its API endpoint.
        blame_enabled: bool = true => Live,
        /// Serve code and commit search (`/search`) and the search box.
        search_enabled: bool = true => Live,
        /// Serve the bytes of a file (`/raw`): downloads and images in rendered Markdown.
        raw_enabled: bool = true => Live,
        /// Serve the history of one file (`/history`).
        history_enabled: bool = true => Live,
        /// Serve the comparison of two revisions (`/compare`).
        compare_enabled: bool = true => Live,
        /// Serve the working tree, linked worktrees and stashes. Never on a public server.
        worktree_enabled: bool = true => Live,
        /// Serve the insights pages (`/stats`) and their API. `insights.pages` picks which.
        insights_enabled: bool = true => Live,
        /// Push changes to open pages (`/events`). Never on a public server.
        live_enabled: bool = true => Live,
        /// Run the actions of `actions.toml` and offer Send to. Never on a public server.
        actions_enabled: bool = true => Live,
        /// Let `git terrace open` and `remove` add and take off repositories of a running
        /// server. Never on a public server. Server-wide.
        repo_admin_enabled: bool = true => Live,
        /// Show the repository switcher and the list of served repositories. Server-wide.
        picker_enabled: bool = true => Live,
        /// Show the front matter of a Markdown file as a table.
        front_matter_enabled: bool = true => Live,
        /// Draw mermaid diagrams in Markdown; off shows their source.
        mermaid_enabled: bool = true => Live,
        /// Write the credit line (source, license, origin) at the foot of exported pages.
        /// Server-wide.
        credit_enabled: bool = true => Live,
        /// Serve the JSON API (`/api/v1`). Unset means on locally and off on a public server.
        /// Server-wide.
        api_enabled: Option<bool> = None => Live,
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
        if self.server.live_streams == 0 {
            return bad("server.live_streams", "must be at least 1");
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
        for (k, lo, hi) in BOUNDS {
            let v = keys()
                .iter()
                .find(|x| x.id == *k)
                .and_then(|x| self.show(x).parse::<u64>().ok());
            if v.is_none_or(|v| v < *lo || v > *hi) {
                return bad(k, &format!("must be between {lo} and {hi}"));
            }
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
        if let Some(s) = self
            .display
            .sidebar
            .iter()
            .find(|s| !SIDEBAR.contains(&s.as_str()))
        {
            return bad(
                "display.sidebar",
                &format!(
                    "unknown section {s}, expected now, recent, churn, loose_ends or languages"
                ),
            );
        }
        let pages = &self.insights.pages;
        if let Some(p) = pages.iter().find(|p| !INSIGHT_PAGES.contains(&p.as_str())) {
            return bad(
                "insights.pages",
                &format!(
                    "unknown page {p}, expected one of {}",
                    INSIGHT_PAGES.join(", ")
                ),
            );
        }
        if let Some(p) = pages
            .iter()
            .enumerate()
            .find(|(i, p)| pages[..*i].contains(p))
        {
            return bad("insights.pages", &format!("{} is listed twice", p.1));
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
    /// `server.is_public` (fails closed, so a typo in `addr` never exposes the working tree), or a
    /// `server.hosts` entry that is not one plain host name.
    fn public_problem(&self) -> Option<(&'static str, String)> {
        let s = &self.server;
        let addr = s.addr.parse::<std::net::SocketAddr>().ok()?;
        if !addr.ip().is_loopback() && !s.is_public {
            return Some((
                "server.addr",
                format!(
                    "{addr} is reachable from other machines. Set server.is_public = true (or pass --public) to serve read-only there"
                ),
            ));
        }
        if !s.hosts.is_empty() && !s.is_public {
            return Some(("server.hosts", "needs server.is_public = true".to_owned()));
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
