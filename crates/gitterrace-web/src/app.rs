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
/// Bytes each of the file list, last commit and rendered page caches may hold. Every entry is
/// cheap to rebuild, so the bound only has to keep a public visitor from pinning memory.
const CACHE_BYTES: usize = 32 << 20;
/// Record sets kept at once, one per selected commit. Each holds all of its history, so this
/// stays small; a visitor cycling more commits than this pays a parse per switch, not a stall
/// of every other page.
const RECORD_SETS: usize = 4;
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
    (
        cfg.limits.text_kb << 10,
        cfg.resources.cache_mb * (1 << 20) / 2,
    )
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
            .filter_map(|t| Some((t.label.clone(), t.lookup_url(sha)?)))
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
        let mut restart = Vec::new();
        for (key, moved) in [
            ("server.addr", new.server.addr != old.server.addr),
            ("index.threads", new.index.threads != old.index.threads),
            ("multi.depth", new.multi.depth != old.multi.depth),
            (
                "multi.skip_dirs",
                new.multi.skip_dirs != old.multi.skip_dirs,
            ),
        ] {
            if moved {
                restart.push(key);
            }
        }
        new.server.addr.clone_from(&old.server.addr);
        new.index.threads = old.index.threads;
        new.multi = old.multi.clone();
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
    totals: Mutex<Option<TotalsEntry>>,
    /// Ignore files of one commit, `(path, text)`. Building the rules from them is cheap; finding
    /// them walks the whole tree.
    ignores: Mutex<Option<IgnoreFiles>>,
    memo: Mutex<crate::bounded::Bounded<(Instant, Arc<String>)>>,
    dirty: Mutex<Option<(Instant, bool)>>,
    /// Open streams on a page that shows the working tree. Zero means no scan runs.
    pub(crate) wt_watchers: std::sync::atomic::AtomicUsize,
    file_lists: Mutex<crate::bounded::Bounded<Arc<Vec<String>>>>,
    last_commits: Mutex<crate::bounded::Bounded<Arc<Vec<LastCommit>>>>,
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
            key: name.clone(),
            name,
            base: String::new(),
            actions: None,
            progress: Mutex::new(Progress::default()),
            ready: AtomicBool::new(false),
            wanted: Mutex::new(std::collections::BTreeSet::new()),
            records: Mutex::new(Vec::new()),
            record_loads: AtomicU64::new(0),
            totals: Mutex::new(None),
            ignores: Mutex::new(None),
            memo: Mutex::new(crate::bounded::Bounded::new(CACHE_BYTES)),
            dirty: Mutex::new(None),
            wt_watchers: std::sync::atomic::AtomicUsize::new(0),
            file_lists: Mutex::new(crate::bounded::Bounded::new(CACHE_BYTES)),
            last_commits: Mutex::new(crate::bounded::Bounded::new(CACHE_BYTES)),
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
    fn reconfigure(&self, cfg: &Config) {
        let (text_bytes, blame_bytes) = repo_limits(cfg);
        self.repo.set_limits(text_bytes, blame_bytes);
        resize(&self.streams, &self.streams_cap, cfg.server.max_streams);
        if let Ok(mut g) = self.memo.lock() {
            g.clear();
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

    /// Size in bytes of the highlighted-code cache: half of `resources.cache_mb`.
    pub(crate) fn code_cache_bytes(&self) -> usize {
        repo_limits(&self.cfg()).1
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
        let (found, complete) = gitterrace_git::last_commits(&self.repo, commit, dir, budget)?;
        let v = Arc::new(found);
        if !complete {
            return Ok(v);
        }
        if let Ok(mut g) = self.last_commits.lock() {
            g.put(key, Arc::clone(&v));
        }
        Ok(v)
    }

    /// Lines of code per language at every first-parent commit of `head`, built once per head.
    pub(crate) fn chain(
        &self,
        head: &str,
        records: &[CommitRecord],
    ) -> Arc<crate::languages::Chain> {
        if let Ok(g) = self.chain.lock()
            && let Some((id, c)) = g.as_ref()
            && id == head
        {
            return Arc::clone(c);
        }
        let ignore = self.ignore_set(head);
        let ins =
            gitterrace_index::Insights::new(records, gitterrace_index::Options::default(), ignore);
        let c = Arc::new(crate::languages::Chain::build(records, &ins));
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
        recs: &[CommitRecord],
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
        let t = gitterrace_index::Insights::new(recs, opts, ignore)
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
        if let Ok(Some(r)) = a.records(&h) {
            a.chain(&h, &r);
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

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;

    use super::App;

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
