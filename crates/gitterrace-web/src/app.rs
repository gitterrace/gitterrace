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
const MEMO_ENTRIES: usize = 64;
/// How long a dirty-worktree answer is reused, so every page need not run a status.
/// The working tree scan runs on every this-many-th watcher tick.
const WORKTREE_EVERY: u64 = 1;
const DIRTY_TTL: Duration = Duration::from_secs(2);
/// Cap on cached directory listings and file lists; both are cheap to rebuild.
const CACHE_ENTRIES: usize = 256;

/// Commit subjects by full id.
pub(crate) type Subjects = HashMap<String, String>;

/// Parsed records keyed by (head, index generation).
type RecordsEntry = ((String, u64), Arc<Vec<CommitRecord>>);

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
}

/// Page-building slots for `cfg`: 0 means one per core.
fn job_slots(cfg: &Config) -> usize {
    match cfg.resources.blocking_jobs {
        0 => std::thread::available_parallelism().map_or(2, usize::from),
        n => n,
    }
}

/// Text cap and blame cache size in bytes for a repository handle.
pub(crate) fn repo_limits(cfg: &Config) -> (u64, usize) {
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
        Self {
            cfg: Arc::new(RwLock::new(Arc::new(cfg))),
            apps: Arc::new(Mutex::new(Vec::new())),
            jobs: Arc::new(Semaphore::new(jobs)),
            jobs_cap: Arc::new(Mutex::new(jobs)),
            stop: Arc::new(watch::channel(false).0),
            actions_note: Arc::default(),
        }
    }

    /// Hands the actions status to every page this server renders.
    pub(crate) fn set_actions_status(&self, status: &Arc<crate::actions::Status>) {
        if let Ok(mut g) = self.actions_note.write() {
            *g = Some(Arc::clone(status));
        }
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
    records: Mutex<Option<RecordsEntry>>,
    memo: Mutex<HashMap<String, (Instant, Arc<String>)>>,
    dirty: Mutex<Option<(Instant, bool)>>,
    /// Open streams on a page that shows the working tree. Zero means no scan runs.
    pub(crate) wt_watchers: std::sync::atomic::AtomicUsize,
    file_lists: Mutex<HashMap<String, Arc<Vec<String>>>>,
    last_commits: Mutex<HashMap<String, Arc<Vec<LastCommit>>>>,
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
            records: Mutex::new(None),
            memo: Mutex::new(HashMap::new()),
            dirty: Mutex::new(None),
            wt_watchers: std::sync::atomic::AtomicUsize::new(0),
            file_lists: Mutex::new(HashMap::new()),
            last_commits: Mutex::new(HashMap::new()),
            chain: Mutex::new(None),
            subjects: Mutex::new(None),
            head_lines: Mutex::new(None),
        }
    }

    /// The settings in force now; a reload can change them between two calls.
    pub(crate) fn cfg(&self) -> Arc<Config> {
        self.env.cfg()
    }

    /// The actions note for this page, rendered with it.
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
    pub(crate) fn set_dirty(&self, v: bool) {
        if let Ok(mut g) = self.dirty.lock() {
            *g = Some((Instant::now(), v));
        }
    }

    /// Drops what a change in the repository can make stale: rendered pages and the dirty chip.
    /// Everything else is keyed by a commit or tree id, which never changes.
    pub(crate) fn invalidate(&self, kinds: &[&str]) {
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
            return Ok(Arc::clone(v));
        }
        let list: Vec<String> = gitterrace_git::files(&self.repo, commit)?
            .into_iter()
            .map(|f| f.path)
            .collect();
        let list = Arc::new(list);
        if let Ok(mut g) = self.file_lists.lock() {
            if g.len() >= CACHE_ENTRIES {
                g.clear();
            }
            g.insert(commit.to_owned(), Arc::clone(&list));
        }
        Ok(list)
    }

    /// Last commit per entry of `dir`, cached by the directory's tree id.
    pub(crate) fn last_commits(&self, commit: &str, dir: &str) -> Result<Arc<Vec<LastCommit>>> {
        let key = gitterrace_git::tree_id(&self.repo, commit, dir)?;
        if let Ok(g) = self.last_commits.lock()
            && let Some(v) = g.get(&key)
        {
            return Ok(Arc::clone(v));
        }
        let v = Arc::new(gitterrace_git::last_commits(
            &self.repo,
            commit,
            dir,
            Duration::from_secs(20),
        )?);
        if let Ok(mut g) = self.last_commits.lock() {
            if g.len() >= CACHE_ENTRIES {
                g.clear();
            }
            g.insert(key, Arc::clone(&v));
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
        let ignore = gitterrace_git::IgnoreSet::at(&self.repo, head).ok();
        let ins =
            gitterrace_index::Insights::new(records, gitterrace_index::Options::default(), ignore);
        let subjects = self.subjects(head);
        let c = Arc::new(crate::languages::Chain::build(records, &ins, &subjects));
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
        let g = self.memo.lock().ok()?;
        let (at, html) = g.get(key)?;
        (at.elapsed() < MEMO_TTL).then(|| Arc::clone(html))
    }

    pub(crate) fn memo_put(&self, key: String, html: Arc<String>) {
        if let Ok(mut g) = self.memo.lock() {
            if g.len() >= MEMO_ENTRIES {
                g.clear();
            }
            g.insert(key, (Instant::now(), html));
        }
    }

    /// The query reduced to what the stats pages read, sorted: unknown parameters and the
    /// order of known ones must not make a new memo entry.
    pub(crate) fn canonical_query(raw: Option<&str>) -> String {
        const KNOWN: [&str; 12] = [
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
        // Held across the load: callers racing on a cold holder wait for the one parse instead
        // of each parsing the whole record file, which multiplied peak memory.
        let Ok(mut g) = self.records.lock() else {
            return Ok(None);
        };
        if let Some((k, recs)) = g.as_ref()
            && *k == key
        {
            return Ok(Some(Arc::clone(recs)));
        }
        if self.index.newer_format().is_some() {
            return Ok(None);
        }
        if self.index.pending(&self.repo, head)? > 0 {
            if let Ok(mut w) = self.wanted.lock() {
                w.insert(head.to_owned());
            }
            self.wake.notify_one();
            return Ok(None);
        }
        // Drop the stale set first so the old and the new one are never alive together.
        *g = None;
        let recs = Arc::new(self.index.records(&self.repo, head)?);
        *g = Some((key, Arc::clone(&recs)));
        Ok(Some(recs))
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
#[allow(clippy::expect_used)]
mod tests {
    use std::process::Command;

    use super::App;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@e.x"])
            .args(args)
            .current_dir(dir)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

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
}
