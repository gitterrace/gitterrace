//! Shared state and the background index refresher.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gitterrace_config::Config;
use gitterrace_git::{LastCommit, Repo};
use gitterrace_index::{CommitRecord, Index, Progress};
use tokio::sync::{Notify, Semaphore, broadcast, watch};

use crate::error::Result;
use crate::live::Change;

/// Rendered stats pages are reused this long; time-relative windows move slowly.
const MEMO_TTL: Duration = Duration::from_secs(60);
/// How long a dirty-worktree answer is reused, so every page need not run a status.
/// The working tree scan runs on every this-many-th watcher tick.
const WORKTREE_EVERY: u64 = 1;
const DIRTY_TTL: Duration = Duration::from_secs(2);
/// Shares of `resources.cache_mb`, in eighths: blame and highlighted code take two each; the
/// rendered pages, file lists, last commits and path logs one each. Together they are the whole
/// budget, so the setting bounds every byte-bounded serving cache of a repository.
const BLAME_EIGHTHS: usize = 2;
const CODE_EIGHTHS: usize = 2;
const SMALL_EIGHTHS: usize = 1;
/// Record sets kept at once, one per selected commit. Each holds all of its history, so this
/// stays small; a visitor cycling more commits than this pays a parse per switch, not a stall
/// of every other page.
const RECORD_SETS: usize = 4;
/// Record sets and the tables built over them are dropped after this long without a page that
/// reads them. They are the bulk of a running server (about 800 MB on a 345k commit history), so
/// an idle server must not keep them; a page after the release parses them again, about 1 s per
/// 350k commits. The page memo still answers the unfiltered pages in between.
pub(crate) const RECORDS_IDLE: Duration = Duration::from_secs(30);
/// Largest ignore file read: these are configuration, never megabytes.
const MAX_IGNORE_BYTES: usize = 1 << 20;

/// Commit subjects by full id.
pub(crate) type Subjects = HashMap<String, String>;

/// One selected commit's records, filled once by whichever caller gets there first.
type RecordSlot = Arc<Mutex<Option<Arc<Vec<CommitRecord>>>>>;

/// Overview totals for one (commit, index generation, options).
/// One commit and its ignore files as `(path, text)`.
type IgnoreFiles = (String, Arc<Vec<(String, String)>>);

type TotalsEntry = (String, gitterrace_index::PeriodStats);

/// What every repository of one server shares: the settings in force, the cap on page-building
/// jobs and the shutdown signal that ends live streams.
#[derive(Clone)]
pub(crate) struct Env {
    cfg: Arc<RwLock<Arc<Config>>>,
    /// Every repository app built so far, so a reload reaches them.
    apps: Arc<Mutex<Vec<Weak<App>>>>,
    jobs: Arc<Semaphore>,
    jobs_cap: Arc<Mutex<usize>>,
    stop: Arc<watch::Sender<bool>>,
    /// Why `actions.toml` is not in force; rendered into every page so the note is there
    /// at first paint instead of arriving with a poll and pushing the page down.
    actions_note: Arc<RwLock<Option<Arc<crate::actions::Status>>>>,
    /// The `actions.toml` Send to targets, reloaded with the file: the commit page reads their
    /// `lookup` links from here.
    sends: Arc<RwLock<Option<SharedActions>>>,
    /// `server.public` at startup. Fixed for the process: a reload never widens what a
    /// public server shows.
    public: bool,
}

/// The live `actions.toml`, swapped whole on reload.
type SharedActions = Arc<RwLock<Arc<gitterrace_actions::Config>>>;

/// Page-building slots for `cfg`: 0 means one per core.
fn job_slots(cfg: &Config) -> usize {
    match cfg.resources.blocking_jobs {
        0 => std::thread::available_parallelism().map_or(2, usize::from),
        n => n,
    }
}

/// Text cap and blame cache size in bytes for a repository handle.
pub(crate) const fn repo_limits(cfg: &Config) -> (u64, usize) {
    (cfg.limits.text_kb << 10, cache_share(cfg, BLAME_EIGHTHS))
}

/// `eighths` of `resources.cache_mb`, in bytes.
const fn cache_share(cfg: &Config, eighths: usize) -> usize {
    (cfg.resources.cache_mb << 20) / 8 * eighths
}

/// Moves a semaphore to `new` slots. Slots in use are not taken back, so a lower cap settles as
/// they are released; `cap` then keeps the number actually in force.
fn resize(sem: &Semaphore, cap: &Mutex<usize>, new: usize) {
    let Ok(mut cap) = cap.lock() else { return };
    if new >= *cap {
        sem.add_permits(new - *cap);
        *cap = new;
    } else {
        *cap -= sem.forget_permits(*cap - new);
    }
}

impl Env {
    pub(crate) fn new(cfg: Config) -> Self {
        let jobs = job_slots(&cfg);
        let public = cfg.server.public;
        Self {
            public,
            cfg: Arc::new(RwLock::new(Arc::new(cfg))),
            apps: Arc::new(Mutex::new(Vec::new())),
            jobs: Arc::new(Semaphore::new(jobs)),
            jobs_cap: Arc::new(Mutex::new(jobs)),
            stop: Arc::new(watch::channel(false).0),
            actions_note: Arc::default(),
            sends: Arc::default(),
        }
    }

    /// Whether this server is public: committed history only, see `public.rs`.
    pub(crate) const fn public(&self) -> bool {
        self.public
    }

    /// Hands the actions status to every page this server renders.
    pub(crate) fn set_actions_status(&self, status: &Arc<crate::actions::Status>) {
        if let Ok(mut g) = self.actions_note.write() {
            *g = Some(Arc::clone(status));
        }
    }

    /// Hands the Send to config to every page this server renders.
    pub(crate) fn set_sends(&self, sends: &SharedActions) {
        if let Ok(mut g) = self.sends.write() {
            *g = Some(Arc::clone(sends));
        }
    }

    /// `(label, url)` for each Send to target with a `lookup`, filled for `sha`.
    pub(crate) fn lookups(&self, sha: &str) -> Vec<(String, String)> {
        let Some(config) = self
            .sends
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(Arc::clone))
        else {
            return Vec::new();
        };
        let Ok(config) = config.read() else {
            return Vec::new();
        };
        config
            .sends
            .iter()
            .filter_map(|t| Some((t.lookup_site(sha)?, t.lookup_url(sha)?)))
            .collect()
    }

    /// The actions note to render now, if any.
    pub(crate) fn actions_note(&self) -> Option<String> {
        self.actions_note.read().ok()?.as_ref()?.note()
    }

    /// The settings in force now.
    pub(crate) fn cfg(&self) -> Arc<Config> {
        self.cfg
            .read()
            .map_or_else(|e| Arc::clone(&e.into_inner()), |g| Arc::clone(&g))
    }

    /// Lets a reload reach `app`.
    pub(crate) fn register(&self, app: &Arc<App>) {
        if let Ok(mut g) = self.apps.lock() {
            g.retain(|w| w.strong_count() > 0);
            g.push(Arc::downgrade(app));
        }
    }

    /// Puts `new` in force. Settings that need a restart keep their running value. Returns the
    /// keys of those the file changed. Every repository drops its rendered pages and tells its
    /// open tabs.
    pub(crate) fn apply(&self, mut new: Config) -> Vec<&'static str> {
        let old = self.cfg();
        // The settings table says which keys are read once at startup (the bind, the Host
        // gate, public mode, discovery). Keeping their running value means `cfg()` never
        // shows a value that is not in force.
        let restart = old.restart_changes(&new);
        new.keep_restart(&old);
        if new == *old {
            return restart;
        }
        let new = Arc::new(new);
        match self.cfg.write() {
            Ok(mut g) => *g = Arc::clone(&new),
            Err(e) => *e.into_inner() = Arc::clone(&new),
        }
        resize(&self.jobs, &self.jobs_cap, job_slots(&new));
        let apps: Vec<Arc<App>> = self
            .apps
            .lock()
            .map(|g| g.iter().filter_map(Weak::upgrade).collect())
            .unwrap_or_default();
        for app in apps {
            app.reconfigure(&new);
        }
        restart
    }

    /// Ends every live stream, so a graceful shutdown does not wait for open tabs.
    pub(crate) fn shut_down(&self) {
        self.stop.send_replace(true);
    }
}

pub(crate) struct App {
    pub(crate) repo: Repo,
    env: Env,
    /// Open `/events` streams, capped by `server.max_streams`.
    pub(crate) streams: Arc<Semaphore>,
    streams_cap: Mutex<usize>,
    /// Change notices for the open streams. Nothing is sent while nobody listens.
    pub(crate) changes: broadcast::Sender<Arc<Change>>,
    pub(crate) stop: watch::Receiver<bool>,
    /// Keeps the shutdown channel open for as long as the app lives.
    _stop_tx: Arc<watch::Sender<bool>>,
    /// Digest of the last fingerprint, so a stream that reconnects can tell it missed something.
    version: AtomicU64,
    /// Wakes the refresher: a revision was queued, or a page asked for the index.
    wake: Notify,
    /// Tells the warm-up that the index moved.
    refreshed: Notify,
    /// A page that needs the index was asked for, see `index.build_on_start`.
    demanded: AtomicBool,
    /// Set when the cache was deleted while serving: the loop rebuilds HEAD even if it did not
    /// move.
    rebuild: AtomicBool,
    pub(crate) index: Index,
    pub(crate) name: String,
    /// URL prefix: empty for a single repository, `/r/NAME` when several share the server.
    pub(crate) base: String,
    /// Name run requests use for this repo: the URL name with several repos.
    pub(crate) key: String,
    /// Set when actions are on: pages embed the token and the menu.
    pub(crate) actions: Option<Arc<crate::actions::Live>>,
    progress: Mutex<Progress>,
    ready: AtomicBool,
    /// Selected commits outside HEAD's history, queued for the one indexer loop so the cache
    /// file never has two writers.
    wanted: Mutex<std::collections::BTreeSet<String>>,
    /// Record sets by (commit, generation), oldest first. The map lock is held only to find a
    /// slot; the parse runs under that slot's own lock, so one cold commit never stalls pages
    /// on another.
    records: Mutex<Vec<((String, u64), RecordSlot)>>,
    /// Record file reads, so a test can tell a hit from a re-read.
    record_loads: AtomicU64,
    /// Last time a page read the record sets, see [`App::release_idle`].
    records_used: Mutex<Instant>,
    /// The insights source of one (commit, generation), so every page shares its derived
    /// tables.
    source: Mutex<Option<((String, u64), crate::insights::Src)>>,
    /// The same source as [`App::source`] holds when it reads the segment files, typed, so a
    /// store error it met turns into a rebuild.
    disk: Mutex<Option<Arc<gitterrace_index::DiskSet>>>,
    totals: Mutex<Option<TotalsEntry>>,
    /// Ignore files of one commit, `(path, text)`. Building the rules from them is cheap; finding
    /// them walks the whole tree.
    ignores: Mutex<Option<IgnoreFiles>>,
    memo: Mutex<crate::bounded::Bounded<(Instant, Arc<String>)>>,
    dirty: Mutex<Option<(Instant, bool)>>,
    /// Open streams on a page that shows the working tree. Zero means no scan runs.
    pub(crate) wt_watchers: std::sync::atomic::AtomicUsize,
    /// This repository's `/api/v1` state, so a reload reaches its settings.
    api: std::sync::OnceLock<gitterrace_api::ApiState>,
    file_lists: Mutex<crate::bounded::Bounded<Arc<Vec<String>>>>,
    last_commits: Mutex<crate::bounded::Bounded<Arc<Vec<LastCommit>>>>,
    /// Ids of the commits `git log -- path` lists, by (tip, path, index generation).
    path_logs: Mutex<crate::bounded::Bounded<Arc<PathLog>>>,
    chain: Mutex<Option<(String, Arc<crate::languages::Chain>)>>,
    subjects: Mutex<Option<(String, Arc<Subjects>)>>,
    head_lines: Mutex<Option<(String, Arc<Vec<gitterrace_index::HeadFile>>)>>,
}

impl App {
    /// An app with the default settings.
    #[cfg(test)]
    pub(crate) fn new(repo: Repo, index: Index) -> Self {
        Self::configured(repo, index, &Env::new(Config::default()))
    }

    pub(crate) fn configured(repo: Repo, index: Index, env: &Env) -> Self {
        let name = repo.name();
        let max_streams = env.cfg().server.max_streams;
        let streams = Arc::new(Semaphore::new(max_streams));
        let small = cache_share(&env.cfg(), SMALL_EIGHTHS);
        Self {
            repo,
            index,
            env: env.clone(),
            streams,
            streams_cap: Mutex::new(max_streams),
            changes: broadcast::channel(16).0,
            stop: env.stop.subscribe(),
            _stop_tx: Arc::clone(&env.stop),
            version: AtomicU64::new(0),
            wake: Notify::new(),
            refreshed: Notify::new(),
            demanded: AtomicBool::new(false),
            rebuild: AtomicBool::new(false),
            key: name.clone(),
            name,
            base: String::new(),
            actions: None,
            progress: Mutex::new(Progress::default()),
            ready: AtomicBool::new(false),
            wanted: Mutex::new(std::collections::BTreeSet::new()),
            records: Mutex::new(Vec::new()),
            record_loads: AtomicU64::new(0),
            records_used: Mutex::new(Instant::now()),
            source: Mutex::new(None),
            disk: Mutex::new(None),
            totals: Mutex::new(None),
            ignores: Mutex::new(None),
            memo: Mutex::new(crate::bounded::Bounded::new(small)),
            dirty: Mutex::new(None),
            wt_watchers: std::sync::atomic::AtomicUsize::new(0),
            api: std::sync::OnceLock::new(),
            file_lists: Mutex::new(crate::bounded::Bounded::new(small)),
            last_commits: Mutex::new(crate::bounded::Bounded::new(small)),
            path_logs: Mutex::new(crate::bounded::Bounded::new(small)),
            chain: Mutex::new(None),
            subjects: Mutex::new(None),
            head_lines: Mutex::new(None),
        }
    }

    /// The settings in force now; a reload can change them between two calls.
    pub(crate) fn cfg(&self) -> Arc<Config> {
        self.env.cfg()
    }

    /// Whether the server is public; see `public.rs`.
    pub(crate) const fn public(&self) -> bool {
        self.env.public()
    }

    /// The actions note for this page, rendered with it.
    pub(crate) fn lookups(&self, sha: &str) -> Vec<(String, String)> {
        if self.public() {
            return Vec::new();
        }
        self.env.lookups(sha)
    }

    pub(crate) fn actions_note(&self) -> Option<String> {
        self.env.actions_note()
    }

    /// Takes a reloaded config: limits, stream cap, rendered pages and open tabs.
    /// Hands the app its `/api/v1` state, once, so a reload reconfigures it too.
    pub(crate) fn set_api(&self, api: gitterrace_api::ApiState) {
        self.api.set(api).ok();
    }

    fn reconfigure(&self, cfg: &Config) {
        if let Some(api) = self.api.get() {
            api.reconfigure(cfg);
        }
        let (text_bytes, blame_bytes) = repo_limits(cfg);
        self.repo.set_limits(text_bytes, blame_bytes);
        resize(&self.streams, &self.streams_cap, cfg.server.max_streams);
        let small = cache_share(cfg, SMALL_EIGHTHS);
        if let Ok(mut g) = self.memo.lock() {
            g.clear();
            g.set_budget(small);
        }
        if let Ok(mut g) = self.file_lists.lock() {
            g.set_budget(small);
        }
        if let Ok(mut g) = self.last_commits.lock() {
            g.set_budget(small);
        }
        if let Ok(mut g) = self.path_logs.lock() {
            g.set_budget(small);
        }
        // The watcher loop re-reads `watch.*` when woken.
        self.wake.notify_one();
        // An error only says no tab is open.
        self.changes
            .send(Arc::new(Change::config(self.version())))
            .ok();
    }

    /// Whether the worktree differs from HEAD, reused for a couple of seconds.
    pub(crate) fn dirty(&self) -> bool {
        if self.public() {
            return false;
        }
        if let Ok(g) = self.dirty.lock()
            && let Some((at, v)) = *g
            && at.elapsed() < DIRTY_TTL
        {
            return v;
        }
        let v = gitterrace_git::is_dirty(&self.repo);
        if let Ok(mut g) = self.dirty.lock() {
            *g = Some((Instant::now(), v));
        }
        v
    }

    /// Records a freshly measured answer so the next event carries it.
    fn set_dirty(&self, v: bool) {
        if let Ok(mut g) = self.dirty.lock() {
            *g = Some((Instant::now(), v));
        }
    }

    /// Drops what a change in the repository can make stale: rendered pages and the dirty chip.
    /// Everything else is keyed by a commit or tree id, which never changes.
    fn invalidate(&self, kinds: &[&str]) {
        let moved = |k: &str| kinds.contains(&k);
        if (moved("head") || moved("refs"))
            && let Ok(mut g) = self.memo.lock()
        {
            g.clear();
        }
        if (moved("head") || moved("refs") || moved("index") || moved("worktree"))
            && let Ok(mut g) = self.dirty.lock()
        {
            *g = None;
        }
    }

    /// Size in bytes of the highlighted-code cache: a quarter of `resources.cache_mb`.
    pub(crate) fn code_cache_bytes(&self) -> usize {
        cache_share(&self.cfg(), CODE_EIGHTHS)
    }

    /// Marks that a page needs the commit index, which starts the build when it waits for one.
    pub(crate) fn demand(&self) {
        if !self.demanded.swap(true, Ordering::AcqRel) {
            self.wake.notify_one();
        }
    }

    /// Digest of the newest fingerprint.
    pub(crate) fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Paths of all files at a full commit id. Commits are immutable, so entries never expire.
    pub(crate) fn file_paths(&self, commit: &str) -> Result<Arc<Vec<String>>> {
        if let Ok(g) = self.file_lists.lock()
            && let Some(v) = g.get(commit)
        {
            return Ok(v);
        }
        let list: Vec<String> = gitterrace_git::files(&self.repo, commit)?
            .into_iter()
            .map(|f| f.path)
            .collect();
        let list = Arc::new(list);
        if let Ok(mut g) = self.file_lists.lock() {
            g.put(commit.to_owned(), Arc::clone(&list));
        }
        Ok(list)
    }

    /// Last commit per entry of `dir`, cached by the walk start and the directory.
    ///
    /// The key is the commit, not the directory's tree id: the same tree can sit in divergent
    /// histories with different last commits. A walk cut short by its budget is returned but not
    /// cached, so the next request walks again instead of serving the gaps forever.
    pub(crate) fn last_commits(&self, commit: &str, dir: &str) -> Result<Arc<Vec<LastCommit>>> {
        self.last_commits_within(commit, dir, Duration::from_secs(20))
    }

    fn last_commits_within(
        &self,
        commit: &str,
        dir: &str,
        budget: Duration,
    ) -> Result<Arc<Vec<LastCommit>>> {
        let dir = dir.trim_matches('/');
        let key = format!("{commit}\0{dir}");
        if let Ok(g) = self.last_commits.lock()
            && let Some(v) = g.get(&key)
        {
            return Ok(v);
        }
        // A ready index names the commits that changed anything under `dir`; the walk visits
        // only those instead of every commit in history.
        let among = self.source(commit)?.map(|src| touched_names(&*src, dir));
        let (found, complete) =
            gitterrace_git::last_commits_among(&self.repo, commit, dir, among.as_deref(), budget)?;
        let v = Arc::new(found);
        if !complete {
            return Ok(v);
        }
        if let Ok(mut g) = self.last_commits.lock() {
            g.put(key, Arc::clone(&v));
        }
        Ok(v)
    }

    /// The commits `git log <commit> -- path` lists, newest first, or `None` while the index is
    /// not ready for `commit` (the caller walks then). Built once per
    /// (commit, path, index generation) from the records, so a repeat costs a map lookup.
    pub(crate) fn path_log(&self, commit: &str, path: &str) -> Result<Option<Arc<PathLog>>> {
        let Some(src) = self.source(commit)? else {
            return Ok(None);
        };
        let key = format!("{commit}\0{path}\0{}", self.index.generation());
        if let Ok(g) = self.path_logs.lock()
            && let Some(v) = g.get(&key)
        {
            return Ok(Some(v));
        }
        let owned = touching_history(&*src, path);
        let history: Vec<(&str, &[String], bool)> = owned
            .iter()
            .map(|(id, parents, t)| (id.as_str(), parents.as_slice(), *t))
            .collect();
        let (ids, complete) = self.repo.touching(path, &history);
        let ids = Arc::new(PathLog { ids, complete });
        if let Ok(mut g) = self.path_logs.lock() {
            g.put(key, Arc::clone(&ids));
        }
        Ok(Some(ids))
    }

    /// Lines of code per language at every first-parent commit of `head`, built once per head.
    pub(crate) fn chain(
        &self,
        head: &str,
        src: &dyn gitterrace_index::feed::InsightsSource,
    ) -> Arc<crate::languages::Chain> {
        if let Ok(g) = self.chain.lock()
            && let Some((id, c)) = g.as_ref()
            && id == head
        {
            return Arc::clone(c);
        }
        let ignore = self.ignore_set(head);
        let ins = gitterrace_index::Insights::from_source(
            src,
            gitterrace_index::Options::default(),
            ignore,
        );
        let c = Arc::new(crate::languages::Chain::from_source(&ins));
        if let Ok(mut g) = self.chain.lock() {
            *g = Some((head.to_owned(), Arc::clone(&c)));
        }
        c
    }

    /// Line count of every file in `head`, built once per head. Reading the line cache,
    /// walking the tree and saving the cache cost tens of ms per insights page otherwise.
    pub(crate) fn head_lines(&self, head: &str) -> Arc<Vec<gitterrace_index::HeadFile>> {
        if let Ok(g) = self.head_lines.lock()
            && let Some((id, f)) = g.as_ref()
            && id == head
        {
            return Arc::clone(f);
        }
        let mut cache = gitterrace_index::LineCache::load(self.index.dir());
        let files = Arc::new(
            gitterrace_index::head_files(&self.repo, head, &mut cache).unwrap_or_default(),
        );
        if let Err(e) = cache.save(self.index.dir()) {
            tracing::warn!("line cache not saved: {e}");
        }
        if let Ok(mut g) = self.head_lines.lock() {
            *g = Some((head.to_owned(), Arc::clone(&files)));
        }
        files
    }

    /// Commit subjects by id for everything reachable from `head`, scanned once per head.
    pub(crate) fn subjects(&self, head: &str) -> Arc<HashMap<String, String>> {
        if let Ok(g) = self.subjects.lock()
            && let Some((id, m)) = g.as_ref()
            && id == head
        {
            return Arc::clone(m);
        }
        let mut map: HashMap<String, String> = HashMap::new();
        if let Err(e) = gitterrace_git::scan_commits(&self.repo, head, |s, _| {
            map.insert(s.id.clone(), s.subject.clone());
            true
        }) {
            tracing::warn!("subjects unavailable: {e}");
        }
        let m = Arc::new(map);
        if let Ok(mut g) = self.subjects.lock() {
            *g = Some((head.to_owned(), Arc::clone(&m)));
        }
        m
    }

    /// A rendered page for `key`, if fresh.
    pub(crate) fn memo_get(&self, key: &str) -> Option<Arc<String>> {
        let (at, html) = self.memo.lock().ok()?.get(key)?;
        (at.elapsed() < MEMO_TTL).then_some(html)
    }

    pub(crate) fn memo_put(&self, key: String, html: Arc<String>) {
        if let Ok(mut g) = self.memo.lock() {
            g.put(key, (Instant::now(), html));
        }
    }

    /// The ignore rules committed at `commit`. The files are found once per commit; the walk
    /// that finds them costs as much as everything else an insights page does.
    pub(crate) fn ignore_set(&self, commit: &str) -> Option<gitterrace_git::IgnoreSet> {
        if let Ok(g) = self.ignores.lock()
            && let Some((id, found)) = g.as_ref()
            && id == commit
        {
            return Some(gitterrace_git::IgnoreSet::from_files(found.iter().cloned()));
        }
        let paths = self.file_paths(commit).ok()?;
        let mut found: Vec<(String, String)> = paths
            .iter()
            .filter(|p| matches!(p.rsplit('/').next(), Some(".gitignore" | ".ignore")))
            .filter_map(|p| {
                let bytes = self.repo.blob_bytes(commit, p).ok()?;
                (bytes.len() <= MAX_IGNORE_BYTES)
                    .then(|| (p.clone(), String::from_utf8_lossy(&bytes).into_owned()))
            })
            .collect();
        // Shallow first: the search tries the last added list first, so deeper rules win.
        found.sort_by_key(|(path, _)| path.matches('/').count());
        let found = Arc::new(found);
        if let Ok(mut g) = self.ignores.lock() {
            *g = Some((commit.to_owned(), Arc::clone(&found)));
        }
        Some(gitterrace_git::IgnoreSet::from_files(found.iter().cloned()))
    }

    /// All-history totals of `commit` with `opts`, computed once per index generation.
    pub(crate) fn totals(
        &self,
        commit: &str,
        src: &dyn gitterrace_index::feed::InsightsSource,
        opts: gitterrace_index::Options,
    ) -> gitterrace_index::PeriodStats {
        let key = format!("{commit}\0{}\0{opts:?}", self.index.generation());
        if let Ok(g) = self.totals.lock()
            && let Some((k, t)) = g.as_ref()
            && *k == key
        {
            return t.clone();
        }
        let ignore = opts
            .respect_ignore
            .then(|| self.ignore_set(commit))
            .flatten();
        let t = gitterrace_index::Insights::from_source(src, opts, ignore)
            .period_summary(gitterrace_index::Period::All)
            .current;
        if let Ok(mut g) = self.totals.lock() {
            *g = Some((key, t.clone()));
        }
        t
    }

    /// The query reduced to what the stats pages read, sorted: unknown parameters and the
    /// order of known ones must not make a new memo entry.
    pub(crate) fn canonical_query(raw: Option<&str>) -> String {
        const KNOWN: [&str; 13] = [
            "type",
            "scope",
            "author",
            "path",
            "period",
            "page",
            "bots",
            "exclude",
            "generated",
            "ignore",
            "ws",
            "base",
            "all",
        ];
        let mut pairs: Vec<&str> = raw
            .unwrap_or_default()
            .split('&')
            .filter(|p| {
                let key = p.split_once('=').map_or(*p, |(k, _)| k);
                KNOWN.contains(&key) || key == "head"
            })
            .collect();
        pairs.sort_unstable();
        pairs.join("&")
    }

    /// Key for a rendered page: moves with HEAD, the selected ref's commit, the index file, the
    /// branch name, the dirty chip and the query, so any of them changing yields a fresh render.
    pub(crate) fn memo_key(&self, tab: &str, query: Option<&str>) -> String {
        format!(
            "{tab}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
            self.base,
            self.repo.resolve("HEAD").unwrap_or_default(),
            self.repo
                .resolve(&crate::fmt::selected())
                .unwrap_or_default(),
            self.index.generation(),
            self.repo.head_branch().unwrap_or_default(),
            self.dirty(),
            query.unwrap_or_default()
        )
    }

    fn set_progress(&self, p: Progress) {
        if let Ok(mut g) = self.progress.lock() {
            *g = p;
        }
    }

    pub(crate) fn progress(&self) -> Progress {
        self.progress.lock().map(|g| *g).unwrap_or_default()
    }

    /// True once one full pass over HEAD finished, so records can be trusted.
    pub(crate) fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub(crate) fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    /// Records reachable from `head`, parsed once per index generation. `None` until the first
    /// build finished, and for a commit outside HEAD's history until its missing commits are
    /// indexed; the indexer loop picks such a commit up from `wanted`.
    pub(crate) fn records(&self, head: &str) -> Result<Option<Arc<Vec<CommitRecord>>>> {
        if !self.ready() {
            self.demand();
            return Ok(None);
        }
        if let Ok(mut g) = self.records_used.lock() {
            *g = Instant::now();
        }
        let key = (head.to_owned(), self.index.generation());
        if let Some(recs) = self.record_slot(&key).and_then(|s| s.lock().ok()?.clone()) {
            return Ok(Some(recs));
        }
        if self.index.newer_format().is_some() {
            return Ok(None);
        }
        // Many distinct commits can land here on a public server; each is indexed once, so the
        // work is bounded by the history, and the slots below stay at `RECORD_SETS`.
        if self.index.pending(&self.repo, head)? > 0 {
            if let Ok(mut w) = self.wanted.lock() {
                w.insert(head.to_owned());
            }
            self.wake.notify_one();
            return Ok(None);
        }
        let slot = self.record_slot_or_new(key);
        // Held across the load: callers racing on one cold commit wait for the one parse instead
        // of each parsing the whole record file, which multiplied peak memory. Other commits
        // have their own slot and do not wait.
        let Ok(mut g) = slot.lock() else {
            return Ok(None);
        };
        if let Some(recs) = g.as_ref() {
            return Ok(Some(Arc::clone(recs)));
        }
        self.record_loads.fetch_add(1, Ordering::Relaxed);
        let recs = Arc::new(self.index.records(&self.repo, head)?);
        *g = Some(Arc::clone(&recs));
        Ok(Some(recs))
    }

    fn record_slot(&self, key: &(String, u64)) -> Option<RecordSlot> {
        let g = self.records.lock().ok()?;
        g.iter().find(|(k, _)| k == key).map(|(_, s)| Arc::clone(s))
    }

    /// What every insights page reads for `head`, `None` while [`App::records`] is. Kept for
    /// one (commit, generation), so a HEAD move or a rebuild never serves a stale source.
    pub(crate) fn source(&self, head: &str) -> Result<Option<crate::insights::Src>> {
        self.check_unreadable()?;
        let key = (head.to_owned(), self.index.generation());
        if let Ok(g) = self.source.lock()
            && let Some((k, s)) = g.as_ref()
            && *k == key
        {
            if let Ok(mut u) = self.records_used.lock() {
                *u = Instant::now();
            }
            return Ok(Some(Arc::clone(s)));
        }
        if !self.ready() {
            self.demand();
            return Ok(None);
        }
        // HEAD (the rev the set was last built for) reads the segment files and holds no
        // history. Another commit falls back to its records until it is the built one.
        if self.index.newer_format().is_none()
            && let Some(set) = self.index.disk_source(&self.repo, head)?
        {
            if let Ok(mut u) = self.records_used.lock() {
                *u = Instant::now();
            }
            if let Ok(mut d) = self.disk.lock() {
                *d = Some(Arc::clone(&set));
            }
            let src: crate::insights::Src = set;
            if let Ok(mut g) = self.source.lock() {
                *g = Some((key, Arc::clone(&src)));
            }
            return Ok(Some(src));
        }
        let Some(recs) = self.records(head)? else {
            return Ok(None);
        };
        let src = gitterrace_index::Index::memory_source(recs);
        if let Ok(mut g) = self.source.lock() {
            *g = Some((key, Arc::clone(&src)));
        }
        Ok(Some(src))
    }

    /// A block that failed its check while serving deletes the cache, drops
    /// every source over it and rebuilds. Until the rebuild is done pages show the indexing
    /// page, never data from a corrupt set.
    fn check_unreadable(&self) -> Result<()> {
        let bad = self
            .disk
            .lock()
            .ok()
            .and_then(|d| d.as_deref().and_then(gitterrace_index::unreadable));
        let Some(why) = bad else {
            return Ok(());
        };
        tracing::warn!("cache unreadable while serving, rebuilding: {why}");
        if let Ok(mut d) = self.disk.lock() {
            d.take();
        }
        if let Ok(mut g) = self.source.lock() {
            g.take();
        }
        self.index.rebuild()?;
        self.ready.store(false, Ordering::Release);
        // HEAD did not move, so the loop would not rebuild it on its own.
        self.rebuild.store(true, Ordering::Release);
        self.wake.notify_one();
        self.demand();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn source_held(&self) -> bool {
        self.source.lock().is_ok_and(|g| g.is_some())
    }

    /// The slot for `key`, made when missing. Sets of an older generation go first, then the
    /// oldest, so at most `RECORD_SETS` stay alive.
    fn record_slot_or_new(&self, key: (String, u64)) -> RecordSlot {
        let Ok(mut g) = self.records.lock() else {
            return Arc::default();
        };
        if let Some((_, s)) = g.iter().find(|(k, _)| *k == key) {
            return Arc::clone(s);
        }
        g.retain(|(k, _)| k.1 == key.1);
        while g.len() >= RECORD_SETS {
            g.remove(0);
        }
        let slot = RecordSlot::default();
        g.push((key, Arc::clone(&slot)));
        slot
    }

    /// Drops the record sets and everything built over them once no page read them for `idle`.
    /// Returns whether it released anything. A page in flight keeps its own `Arc`, so it is
    /// never cut short; the next page parses again. Rendered pages, file lists and other small
    /// byte-bounded caches stay.
    pub(crate) fn release_idle(&self, idle: Duration) -> bool {
        let quiet = self.records_used.lock().is_ok_and(|g| g.elapsed() >= idle);
        if !quiet {
            return false;
        }
        let held = self.records.lock().map(|mut g| {
            let n = g.len();
            g.clear();
            n
        });
        let mut released = matches!(held, Ok(n) if n > 0);
        // A source over records (a commit the segment set does not hold) holds that history,
        // and the segment one holds open files and block caches: both go.
        released |= self.source.lock().is_ok_and(|mut g| g.take().is_some());
        if let Ok(mut d) = self.disk.lock() {
            d.take();
        }
        for done in [
            self.chain.lock().map(|mut g| g.take().is_some()).ok(),
            self.subjects.lock().map(|mut g| g.take().is_some()).ok(),
            self.head_lines.lock().map(|mut g| g.take().is_some()).ok(),
        ] {
            released |= done.unwrap_or(false);
        }
        gitterrace_index::Insights::release_shared(&format!("{}\u{0}", self.base));
        // The API parses its own set; a caller there is a client that can wait a second too.
        if let Some(api) = self.api.get() {
            api.release_records();
        }
        released
    }

    /// How often the record file was read, for tests.
    #[cfg(test)]
    pub(crate) fn record_loads(&self) -> u64 {
        self.record_loads.load(Ordering::Relaxed)
    }
}

/// Runs git or index work off the async threads.
pub(crate) async fn blocking<T: Send + 'static>(
    app: &Arc<App>,
    f: impl FnOnce(&App) -> Result<T> + Send + 'static,
) -> Result<T> {
    let app = Arc::clone(app);
    let rev = crate::fmt::REQUEST_REV
        .try_with(Clone::clone)
        .unwrap_or_else(|_| "HEAD".to_owned());
    // Held until the work ends: `resources.blocking_jobs` bounds git work, not waiting.
    let _slot = Arc::clone(&app.env.jobs).acquire_owned().await.ok();
    let absolute = app.cfg().display.dates == "absolute";
    tokio::task::spawn_blocking(move || {
        crate::fmt::with_dates(absolute, || {
            crate::fmt::with_base(&app.base, || crate::fmt::with_rev(&rev, || f(&app)))
        })
    })
    .await?
}

impl App {
    /// Reacts to the parts of the repository that moved: drops stale caches and, when a tab is
    /// listening, tells it. Costs nothing but the cache reset while nobody listens.
    async fn changed(self: &Arc<Self>, kinds: Vec<&'static str>, v: u64) {
        self.invalidate(&kinds);
        self.version.store(v, Ordering::Release);
        if self.changes.receiver_count() == 0 {
            return;
        }
        let change = blocking(self, move |a| {
            Ok(Change {
                kinds,
                v,
                banner: crate::views::banner(&a.repo.head_state(), &a.base),
                dirty: a.dirty(),
            })
        })
        .await;
        if let Ok(c) = change {
            // An error only says every listener left meanwhile.
            self.changes.send(Arc::new(c)).ok();
        }
    }
}

/// Keeps the commit index and the open tabs current. One fingerprint of cheap `stat` calls per
/// `watch.interval_ms` says whether anything moved; only then is HEAD resolved again. With the
/// watcher off nothing runs on a timer: a page that finds the index behind queues a refresh.
pub(crate) async fn keep_index_fresh(app: Arc<App>) {
    let mut watcher: Option<gitterrace_git::Watcher> = None;
    let mut seen = None;
    let mut last: Option<String> = None;
    let mut check_head = true;
    let mut wt_seen: Option<u64> = None;
    let mut dirty_seen: Option<bool> = None;
    let mut ticks: u64 = 0;
    loop {
        // With `index.build_on_start` off the index waits for the first page that needs it;
        // the watcher below keeps running either way.
        // `watch.*` can change while running: the interval and the on switch are read per round.
        let cfg = app.cfg();
        let every = cfg
            .watch
            .enabled
            .then(|| Duration::from_millis(cfg.watch.interval_ms));
        if every.is_none() {
            watcher = None;
            seen = None;
        } else if watcher.is_none() {
            let mut w = app.repo.watcher();
            let p = w.parts();
            app.version.store(p.digest(), Ordering::Release);
            seen = Some(p);
            watcher = Some(w);
        }
        if cfg.index.build_on_start || app.demanded.load(Ordering::Acquire) {
            if app.rebuild.swap(false, Ordering::AcqRel) {
                check_head = true;
                last = None;
            }
            if check_head {
                check_head = false;
                let head = blocking(&app, |a| Ok(a.repo.resolve("HEAD")?)).await.ok();
                if head.is_some() && head != last {
                    refresh(&app, "HEAD", head.as_deref()).await;
                    last = head;
                }
            }
            let wanted = app
                .wanted
                .lock()
                .map(|mut w| std::mem::take(&mut *w))
                .unwrap_or_default();
            for rev in wanted {
                refresh(&app, &rev, None).await;
            }
        }
        let tick = async {
            match every {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = app.wake.notified() => {}
            () = tick => {
                let Some(w) = watcher.as_mut() else { continue };
                let now = w.parts();
                let mut kinds = seen
                    .filter(|b| *b != now)
                    .map(|before| now.changed_since(&before))
                    .unwrap_or_default();
                ticks = ticks.wrapping_add(1);
                // The working tree scan runs only while a page shows it.
                if app.wt_watchers.load(Ordering::Acquire) == 0 {
                    wt_seen = None;
                    // Any other page still shows the top bar chip: one early-exit check per
                    // tick, and an event only when the answer flips.
                    if let Some(flip) = chip_flip(&app, &mut dirty_seen).await
                        && kinds.is_empty()
                    {
                        app.set_dirty(flip);
                        kinds.push("dirty");
                    }
                } else if ticks.is_multiple_of(WORKTREE_EVERY) {
                    dirty_seen = None;
                    let a = Arc::clone(&app);
                    let sig = tokio::task::spawn_blocking(move || {
                        gitterrace_git::worktree_signature(&a.repo).ok()
                    })
                    .await
                    .ok()
                    .flatten();
                    if wt_seen.is_some() && sig.is_some() && sig != wt_seen {
                        kinds.push("worktree");
                    }
                    if sig.is_some() {
                        wt_seen = sig;
                    }
                }
                if !kinds.is_empty() {
                    check_head |= kinds.iter().any(|k| matches!(*k, "head" | "refs"));
                    app.changed(kinds, now.digest()).await;
                }
                seen = Some(now);
            }
        }
    }
}

/// The early-exit dirty check for the top bar chip, run only while a tab listens. Returns the
/// new answer when it differs from the last one seen.
async fn chip_flip(app: &Arc<App>, seen: &mut Option<bool>) -> Option<bool> {
    if app.changes.receiver_count() == 0 {
        *seen = None;
        return None;
    }
    let a = Arc::clone(app);
    let now = tokio::task::spawn_blocking(move || gitterrace_git::is_dirty(&a.repo))
        .await
        .ok()?;
    let flipped = seen.is_some_and(|b| b != now);
    *seen = Some(now);
    flipped.then_some(now)
}

/// Brings the index up to `rev`; for HEAD (`warm` holds its id) also builds what the first
/// visitor would otherwise wait for, then tells the warm-up.
async fn refresh(app: &Arc<App>, rev: &str, warm: Option<&str>) {
    let a = Arc::clone(app);
    let spec = rev.to_owned();
    let run =
        tokio::task::spawn_blocking(move || a.index.update(&a.repo, &spec, &|p| a.set_progress(p)))
            .await;
    match run {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return tracing::warn!("index update for {rev} failed: {e}"),
        Err(e) => return tracing::warn!("index task failed: {e}"),
    }
    let Some(head) = warm else { return };
    app.mark_ready();
    let (a, h) = (Arc::clone(app), head.to_owned());
    // Warm what the first visitor would otherwise pay for.
    tokio::task::spawn_blocking(move || {
        if let Ok(Some(r)) = a.source(&h) {
            a.chain(&h, &*r);
        }
    });
    app.refreshed.notify_one();
}

/// Waits for the index to move, then runs `warm` once per move.
pub(crate) async fn on_refreshed<F, Fut>(app: Arc<App>, mut warm: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    loop {
        app.refreshed.notified().await;
        warm().await;
    }
}

/// Releases the record sets of an app nobody browses, see [`App::release_idle`]. Checks a few
/// times per idle period, so the release lands at most a quarter period late. Holds only a weak
/// reference, so a removed repository's app is dropped and the loop ends.
pub(crate) async fn release_when_idle(app: Weak<App>, idle: Duration) {
    let mut tick = tokio::time::interval(idle / 4);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(a) = app.upgrade() else { return };
        if a.release_idle(idle) {
            tracing::debug!("{}: released idle record sets", a.name);
        }
    }
}

/// A path's log from the index: `ids` newest first. When not `complete`, an unreadable commit
/// cut it short and only the pages inside `ids` may be served from it.
pub(crate) struct PathLog {
    pub(crate) ids: Vec<String>,
    pub(crate) complete: bool,
}

impl crate::bounded::Weight for PathLog {
    fn weight(&self) -> usize {
        self.ids.weight()
    }
}

/// Every commit of the records, newest first (records are oldest first, in the walk's order
/// reversed), with its parents and whether its first-parent diff touches a path at or under
/// `path`. A commit that does not holds `path` as its first parent does.
fn touching_history(
    src: &dyn gitterrace_index::feed::InsightsSource,
    path: &str,
) -> Vec<(String, Vec<String>, bool)> {
    use gitterrace_index::feed::CommitRange;
    let ranks: Vec<_> = src.commits_in(CommitRange::All).map(|c| c.rank).collect();
    let mut out: Vec<(String, Vec<String>, bool)> = src
        .rows_for(ranks)
        .map(|(rank, rows)| {
            let touched = path.is_empty()
                || rows.iter().any(|f| {
                    src.path(f.path)
                        .is_some_and(|p| crate::filter::under(&p, path))
                        || f.old
                            .and_then(|o| src.path(o))
                            .is_some_and(|o| crate::filter::under(&o, path))
                });
            let id = src
                .commit_id(rank)
                .as_deref()
                .unwrap_or_default()
                .to_owned();
            let parents = src.parent_ids(rank).iter().map(|p| p.to_string()).collect();
            (id, parents, touched)
        })
        .collect();
    out.reverse();
    out
}

/// For each commit whose first-parent diff touches something under `dir`, newest first, the
/// names of the entries of `dir` it touches (old and new side of a rename).
fn touched_names(
    src: &dyn gitterrace_index::feed::InsightsSource,
    dir: &str,
) -> Vec<(String, Vec<String>)> {
    use gitterrace_index::feed::CommitRange;
    let name = |p: &str| -> Option<String> {
        let rest = if dir.is_empty() {
            p
        } else {
            p.strip_prefix(dir)?.strip_prefix('/')?
        };
        Some(rest.split('/').next().unwrap_or(rest).to_owned())
    };
    let ranks: Vec<_> = src.commits_in(CommitRange::All).map(|c| c.rank).collect();
    let mut out: Vec<(String, Vec<String>)> = src
        .rows_for(ranks)
        .filter_map(|(rank, rows)| {
            let mut names: Vec<String> = rows
                .iter()
                .flat_map(|f| [Some(f.path), f.old])
                .flatten()
                .filter_map(|id| src.path(id))
                .filter_map(|p| name(&p))
                .collect();
            names.sort_unstable();
            names.dedup();
            let id = src.commit_id(rank)?.to_string();
            (!names.is_empty()).then_some((id, names))
        })
        .collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use gitterrace_testkit::git;

    use super::{App, LastCommit};

    #[test]
    fn junk_and_order_do_not_make_new_memo_keys() {
        let canon = |q| App::canonical_query(Some(q));
        assert_eq!(canon("x=1&type=fix"), canon("type=fix&x=2&y"));
        assert_eq!(canon("period=1y&type=fix"), canon("type=fix&period=1y"));
        assert_ne!(canon("type=fix"), canon("type=feat"));
        assert_eq!(App::canonical_query(None), "");
    }

    #[test]
    fn memo_key_moves_with_head_and_index_and_records_follow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        std::fs::write(path.join("a.txt"), "1\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "feat: one"]);
        let repo = gitterrace_git::Repo::discover(path).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let key = app.memo_key("t", Some("type=fix"));
        assert_ne!(key, app.memo_key("t", Some("type=feat")));
        app.memo_put(key.clone(), std::sync::Arc::new("page".into()));
        assert!(app.memo_get(&key).is_some());
        let first = app.records(&head).expect("records").expect("ready");
        assert_eq!(first.len(), 1);

        std::fs::write(path.join("a.txt"), "2\n").expect("write");
        git(path, &["commit", "-qam", "fix: two"]);
        let moved = app.memo_key("t", Some("type=fix"));
        assert_ne!(key, moved, "moving HEAD must change the memo key");
        assert!(app.memo_get(&moved).is_none());
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let second = app.records(&head).expect("records").expect("ready");
        assert_eq!(second.len(), 2, "records cache must follow the new tip");
    }

    /// Overview totals are an all-history pass (about 400 ms per request on rust-lang/rust
    /// before the memo). A repeat at the same tip must not read the records again: it is
    /// handed none here, so any recomputation counts zero commits.
    #[test]
    fn overview_totals_are_computed_once_per_tip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        std::fs::write(path.join("a.txt"), "1\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "feat: one"]);
        let repo = gitterrace_git::Repo::discover(path).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let opts = || app.defaults().into_options();
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        assert_eq!(app.totals(&head, &*src, opts()).commits, 1);
        let empty = gitterrace_index::feed::MemorySource::new(&[]);
        assert_eq!(
            app.totals(&head, &empty, opts()).commits,
            1,
            "repeat must hit the memo"
        );

        std::fs::write(path.join("a.txt"), "2\n").expect("write");
        git(path, &["commit", "-qam", "fix: two"]);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        assert_eq!(
            app.totals(&head, &*src, opts()).commits,
            2,
            "a new tip recomputes"
        );
    }

    #[test]
    fn racing_callers_share_one_parsed_record_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        std::fs::write(path.join("a.txt"), "1\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "feat: one"]);
        let repo = gitterrace_git::Repo::discover(path).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = std::sync::Arc::new(App::new(repo, index));
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let sets: Vec<_> = (0..8)
            .map(|_| {
                let (a, h) = (std::sync::Arc::clone(&app), head.clone());
                std::thread::spawn(move || a.records(&h).expect("records").expect("ready"))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().expect("join"))
            .collect();
        assert!(
            sets.iter().all(|s| std::sync::Arc::ptr_eq(s, &sets[0])),
            "every racing caller must get the one parsed set"
        );
    }

    /// A block that fails its check while pages are served is never served
    /// as data. The next page deletes the cache, gets no source (the indexing page) and the
    /// loop rebuilds. Red without `check_unreadable`: the corrupt set keeps serving.
    #[test]
    fn a_block_corrupted_while_serving_rebuilds_the_cache() {
        use gitterrace_index::feed::CommitRange;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        std::fs::write(path.join("a.txt"), "1\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "feat: one"]);
        let repo = gitterrace_git::Repo::discover(path).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        assert!(
            app.disk.lock().is_ok_and(|d| d.is_some()),
            "HEAD reads the segments"
        );
        let cache = app.index.dir().to_path_buf();
        let seg = std::fs::read_dir(&cache)
            .expect("ls")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.extension().is_some_and(|x| x == "gts")
                    && p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("seg-"))
            })
            .expect("a segment");
        // Zero every block byte (same length, so open-time checks still pass).
        let mut bytes = std::fs::read(&seg).expect("read");
        let end = bytes.len() - 64;
        let toc = usize::try_from(u64::from_le_bytes(
            bytes[end + 16..end + 24].try_into().expect("8 bytes"),
        ))
        .expect("toc");
        bytes[16..toc].fill(0);
        std::fs::write(&seg, &bytes).expect("corrupt");
        // A page reads the tables and the rows: the zeroed blocks fail their crc.
        let read = src.agg_author_day(i64::MIN..i64::MAX).count()
            + src.agg_path_author("").count()
            + src.rename_edges().count()
            + src.commits_in(CommitRange::All).count();
        assert_eq!(read, 0, "nothing of a corrupt block is handed out");
        drop(src);
        assert!(
            app.source(&head).expect("source").is_none(),
            "a corrupt set must not serve"
        );
        assert!(
            !cache.join("MANIFEST").exists(),
            "the cache is deleted for a rebuild"
        );
    }

    #[test]
    fn alternating_selected_commits_reuse_their_parsed_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        std::fs::write(path.join("a.txt"), "1\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "feat: one"]);
        let first = git(path, &["rev-parse", "HEAD"]);
        std::fs::write(path.join("a.txt"), "2\n").expect("write");
        git(path, &["commit", "-qam", "fix: two"]);
        let repo = gitterrace_git::Repo::discover(path).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        for _ in 0..3 {
            assert_eq!(
                app.records(&head).expect("records").expect("ready").len(),
                2
            );
            assert_eq!(
                app.records(&first).expect("records").expect("ready").len(),
                1
            );
        }
        assert_eq!(app.record_loads(), 2, "each commit's records are read once");
    }

    /// The parsed history is the bulk of a running server, so nothing may keep it alive once
    /// nobody browses: not the record slots, the shared insights tables, the language chain or
    /// any cache added later. The test holds only a weak reference, so whatever pins the set
    /// turns it red; a page in flight keeps its own `Arc` and the next page parses again.
    #[test]
    fn an_idle_app_lets_go_of_its_records_and_parses_them_again_on_use() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        std::fs::write(path.join("a.rs"), "fn a() {}\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "feat: one"]);
        let repo = gitterrace_git::Repo::discover(path).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let recs = app.records(&head).expect("records").expect("ready");
        // Everything the pages build over the records, as the warm-up does.
        let src = app.source(&head).expect("source").expect("ready");
        app.chain(&head, &*src);
        crate::fmt::with_rev(&head, || {
            let _ins = crate::insights::insights(&app, &src, gitterrace_index::Options::default());
        });
        drop(src);
        assert!(app.source_held());
        let weak = Arc::downgrade(&recs);
        drop(recs);
        // The tables hold no `Arc` of the records (they index into them), so count them too.
        let tables = || gitterrace_index::Insights::shared_sets(&format!("\u{0}{head}"));
        assert_eq!(tables(), 1);
        assert!(!app.release_idle(Duration::from_secs(3600)), "in use");
        assert!(weak.upgrade().is_some(), "kept while browsed");
        assert!(app.release_idle(Duration::ZERO));
        assert!(
            weak.upgrade().is_none(),
            "an idle app still holds its records"
        );
        assert_eq!(
            tables(),
            0,
            "an idle app still holds the tables built over them"
        );
        assert!(
            !app.source_held(),
            "an idle app still holds its insights source"
        );
        let again = app.records(&head).expect("records").expect("ready");
        assert_eq!(again.len(), 1);
        assert_eq!(app.record_loads(), 2, "parsed again after the release");
    }

    /// A history with renames, a dir rename, deletes, an untouched-path merge, a merge that
    /// takes one side, and an evil merge that matches neither parent.
    fn tangled() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        let mut day = 0;
        let mut commit = |msg: &str| {
            day += 1;
            let date = format!("2024-01-{day:02}T12:00:00Z");
            gitterrace_testkit::git_at(p, &date, &["add", "-A"]);
            gitterrace_testkit::git_at(p, &date, &["commit", "-q", "--allow-empty", "-m", msg]);
            date
        };
        let w = |f: &str, body: &str| {
            let f = p.join(f);
            std::fs::create_dir_all(f.parent().expect("parent")).expect("mkdir");
            std::fs::write(f, body).expect("write");
        };
        git(p, &["init", "-q", "-b", "main"]);
        w("d/a", "1\n");
        w("d/b", "1\n");
        w("e/c", "1\n");
        commit("base");
        w("d/a", "2\n");
        commit("a two");
        git(p, &["checkout", "-q", "-b", "side"]);
        w("d/b", "side\n");
        w("e/c", "side\n");
        commit("side b");
        git(p, &["checkout", "-q", "main"]);
        w("d/a", "3\n");
        commit("a three");
        let date = commit("unused");
        gitterrace_testkit::git_at(p, &date, &["merge", "-q", "--no-ff", "--no-commit", "side"]);
        w("d/b", "evil\n");
        commit("evil merge");
        git(p, &["mv", "d/a", "d/a2"]);
        commit("rename a");
        git(p, &["checkout", "-q", "-b", "other", "HEAD~3"]);
        w("e/c", "other\n");
        commit("other c");
        git(p, &["checkout", "-q", "main"]);
        let date = commit("empty");
        let theirs = ["-X", "theirs", "-m", "merge other", "other"];
        let merge = [&["merge", "-q", "--no-ff"][..], &theirs].concat();
        gitterrace_testkit::git_at(p, &date, &merge);
        git(p, &["mv", "e", "f"]);
        commit("move e");
        std::fs::remove_file(p.join("d/b")).expect("rm");
        commit("drop b");
        // A side branch whose commit is dated long before its parent (rust has these), then
        // merged: a date-ordered walk meets it after commits it should have hidden.
        git(p, &["checkout", "-q", "-b", "skew"]);
        w("d/a2", "skew\n");
        let old = "2023-06-01T12:00:00Z";
        gitterrace_testkit::git_at(p, old, &["commit", "-qam", "skewed a"]);
        git(p, &["checkout", "-q", "main"]);
        w("f/c", "late\n");
        commit("late c");
        let date = commit("unused2");
        let merge = ["merge", "-q", "--no-ff", "-m", "merge skew", "skew"];
        gitterrace_testkit::git_at(p, &date, &merge);
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        (dir, app)
    }

    /// The indexed path log and last-commit walk must list exactly what the full history walk
    /// lists, for files, dirs, renamed, moved, deleted and merged paths, at every commit.
    #[test]
    fn indexed_path_history_matches_the_walk() {
        let (dir, app) = tangled();
        let all = git(dir.path(), &["rev-list", "HEAD"]);
        let paths = ["d", "d/a", "d/a2", "d/b", "e", "e/c", "f", "f/c", "nope"];
        for tip in all.lines() {
            for path in paths {
                let walked: Vec<String> = app
                    .repo
                    .log(tip, Some(path), 0, usize::MAX)
                    .expect("log")
                    .into_iter()
                    .map(|c| c.id)
                    .collect();
                let log = app.path_log(tip, path).expect("path log").expect("ready");
                assert!(log.complete);
                assert_eq!(log.ids, walked, "path {path} at {tip}");
                // Users compare against git itself: its default history simplification.
                let real = git(dir.path(), &["log", "--format=%H", tip, "--", path]);
                let real: Vec<String> = real.lines().map(str::to_owned).collect();
                assert_eq!(walked, real, "git log -- {path} at {tip}");
            }
            for d in ["", "d", "e", "f"] {
                let names = |v: Vec<LastCommit>| -> Vec<_> {
                    v.into_iter().map(|l| (l.name, l.id)).collect()
                };
                let walked = gitterrace_git::last_commits(&app.repo, tip, d, Duration::MAX)
                    .expect("walk")
                    .0;
                let indexed = app.last_commits(tip, d).expect("indexed");
                let indexed = names((*indexed).clone());
                assert_eq!(indexed, names(walked), "dir {d:?} at {tip}");
            }
        }
    }

    /// A path's history is answered once per (tip, path, index generation): a repeat must not
    /// filter history again. Seen red with the `path_logs` put removed.
    #[test]
    fn path_log_is_memoized_per_tip_and_generation() {
        let (dir, app) = tangled();
        let head = app.repo.resolve("HEAD").expect("head");
        let first = app.path_log(&head, "d/b").expect("log").expect("ready");
        let again = app.path_log(&head, "d/b").expect("log").expect("ready");
        assert!(Arc::ptr_eq(&first, &again), "a repeat must hit the memo");
        std::fs::write(dir.path().join("d/a2"), "4\n").expect("write");
        git(dir.path(), &["commit", "-qam", "four"]);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        let head2 = app.repo.resolve("HEAD").expect("head");
        let moved = app.path_log(&head2, "d/a2").expect("log").expect("ready");
        let newest = moved.ids.first();
        assert_eq!(newest, Some(&head2), "a new tip lists its own commit");
    }

    /// A commit whose tree cannot be read makes the indexed log give up (`complete` false, no
    /// ids), so the route walks as before instead of serving a list with a gap.
    #[test]
    fn a_missing_tree_cuts_the_path_log_to_a_correct_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        for n in 0..6 {
            std::fs::write(p.join("f"), format!("{n}\n")).expect("write");
            std::fs::write(p.join("g"), format!("{n}\n")).expect("write");
            git(p, &["add", "."]);
            git(p, &["commit", "-q", "-m", &format!("c{n}")]);
        }
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let walked = app
            .repo
            .log(&head, Some("f"), 0, 3)
            .expect("log")
            .into_iter()
            .len();
        let spec = format!("HEAD~4^{}tree{}", '{', '}');
        let tree = git(p, &["rev-parse", &spec]);
        let (dir2, file) = tree.trim().split_at(2);
        std::fs::remove_file(p.join(".git/objects").join(dir2).join(file)).expect("rm tree");
        let log = app.path_log(&head, "f").expect("log").expect("ready");
        assert!(!log.complete, "the cut must be reported");
        assert!(log.ids.is_empty(), "no partial list is kept");
        assert_eq!(walked, 3, "the walk still answers the newer commits");
    }

    #[test]
    fn last_commit_cache_follows_history_not_tree_and_skips_cut_walks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        let commit = |msg: &str, date: &str| {
            gitterrace_testkit::git_at(path, date, &["add", "-A"]);
            gitterrace_testkit::git_at(path, date, &["commit", "-q", "-m", msg]);
            git(path, &["rev-parse", "HEAD"])
        };
        git(path, &["init", "-q", "-b", "main"]);
        std::fs::create_dir(path.join("d")).expect("mkdir");
        std::fs::write(path.join("d/f"), "1\n").expect("write");
        commit("feat: base", "2024-01-01T12:00:00Z");
        git(path, &["checkout", "-q", "-b", "side"]);
        std::fs::write(path.join("d/f"), "2\n").expect("write");
        let on_side = commit("feat: side", "2024-02-01T12:00:00Z");
        git(path, &["checkout", "-q", "main"]);
        std::fs::write(path.join("d/f"), "2\n").expect("write");
        let on_main = commit("feat: main", "2024-03-01T12:00:00Z");
        let repo = gitterrace_git::Repo::discover(path).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        let cut = app
            .last_commits_within(&on_main, "d", std::time::Duration::ZERO)
            .expect("cut");
        assert!(cut.is_empty());
        let main = app.last_commits(&on_main, "d").expect("main");
        assert_eq!(main.first().map(|c| c.id.as_str()), Some(on_main.as_str()));
        let side = app.last_commits(&on_side, "d").expect("side");
        assert_eq!(side.first().map(|c| c.id.as_str()), Some(on_side.as_str()));
    }
}
