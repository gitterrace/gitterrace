//! Shared state and the background index refresher.

use std::{
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gitterrace_config::{Cache, Config};
use gitterrace_git::{LastCommit, Repo};
use gitterrace_index::{Index, Progress};
use tokio::sync::{Notify, Semaphore, broadcast, watch};

use crate::error::Result;
use crate::live::Change;

/// Rendered stats pages are reused this long; time-relative windows move slowly.
const MEMO_TTL: Duration = Duration::from_secs(60);
/// How long a dirty-worktree answer is reused, so every page need not run a status.
pub(crate) const DIRTY_TTL: Duration = Duration::from_secs(2);
/// The source, its block cache and the tables built over it are dropped after this long without
/// a page that reads them, so an idle server keeps none of them; a page after the release opens
/// the set again. The page memo still answers the unfiltered pages in between.
pub(crate) const RECORDS_IDLE: Duration = Duration::from_secs(30);
/// Largest ignore file read: these are configuration, never megabytes.
/// Not configurable: a read bound on a file from the repository, which may be hostile.
const MAX_IGNORE_BYTES: usize = 1 << 20;

mod events;
mod history;
mod memo;
mod records;
mod refresh;
mod touched;

pub(crate) use history::PathLog;
pub(crate) use refresh::{keep_index_fresh, on_refreshed, release_when_idle};

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
    /// The `actions.toml` Send to targets, reloaded with the file, read by the commit page.
    sends: Arc<RwLock<Option<SharedActions>>>,
    /// `server.is_public` at startup. Fixed for the process: a reload never widens what a
    /// public server shows.
    public: bool,
    /// An export run's memo of commit tree diffs, handed to every repository it opens. `None`
    /// for a server, which keeps no per-commit state outside its budgeted caches.
    pub(crate) changes: Option<Arc<gitterrace_git::ChangeMemo>>,
    /// The config folder the settings page edits. `None` (an export, a test, no config folder)
    /// leaves its routes unmounted.
    pub(crate) settings_dir: Option<std::path::PathBuf>,
    /// The folders the server was started on to search for repositories. A sweep keeps the
    /// table in step with them (see `multi/sweep.rs`); empty for named repositories only.
    pub(crate) search: Vec<std::path::PathBuf>,
    /// Whether the settings routes are mounted, so pages offer the settings link only then.
    settings_on: Arc<AtomicBool>,
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
pub(crate) fn repo_limits(cfg: &Config) -> (u64, usize) {
    (cfg.limits.text_kb << 10, cfg.cache_bytes(Cache::Blame))
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
        let public = cfg.server.is_public;
        Self {
            public,
            cfg: Arc::new(RwLock::new(Arc::new(cfg))),
            apps: Arc::new(Mutex::new(Vec::new())),
            jobs: Arc::new(Semaphore::new(jobs)),
            jobs_cap: Arc::new(Mutex::new(jobs)),
            stop: Arc::new(watch::channel(false).0),
            actions_note: Arc::default(),
            sends: Arc::default(),
            changes: None,
            settings_dir: None,
            search: Vec::new(),
            settings_on: Arc::default(),
        }
    }

    /// Records that the settings routes are mounted.
    pub(crate) fn set_settings_on(&self) {
        self.settings_on.store(true, Ordering::Release);
    }

    /// Whether pages offer the settings page.
    pub(crate) fn settings_on(&self) -> bool {
        self.settings_on.load(Ordering::Acquire)
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

    /// Tells every open tab that the set of served repositories changed, so the switcher shows
    /// a repository that joined without a reload.
    pub(crate) fn repos_changed(&self) {
        let apps: Vec<Arc<App>> = self
            .apps
            .lock()
            .map(|g| g.iter().filter_map(Weak::upgrade).collect())
            .unwrap_or_default();
        for app in apps {
            // An error only says no tab is open.
            app.changes
                .send(Arc::new(Change::repos(app.version())))
                .ok();
        }
    }

    /// The settings in force now.
    pub(crate) fn cfg(&self) -> Arc<Config> {
        self.cfg
            .read()
            .map_or_else(|e| Arc::clone(&e.into_inner()), |g| Arc::clone(&g))
    }

    /// Every live app built on this environment.
    #[cfg(test)]
    pub(crate) fn apps_for_test(&self) -> Vec<Arc<App>> {
        self.apps
            .lock()
            .map(|g| g.iter().filter_map(Weak::upgrade).collect())
            .unwrap_or_default()
    }

    /// How many repositories are served: more than one shows the switcher from the first paint.
    pub(crate) fn served(&self) -> usize {
        self.apps
            .lock()
            .map_or(1, |g| g.iter().filter(|w| w.strong_count() > 0).count())
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
    pub(crate) env: Env,
    /// Open `/events` streams, capped by `server.live_streams`.
    pub(crate) streams: Arc<Semaphore>,
    streams_cap: Mutex<usize>,
    /// Change notices for the open streams. Nothing is sent while nobody listens.
    pub(crate) changes: broadcast::Sender<Arc<Change>>,
    pub(crate) stop: watch::Receiver<bool>,
    /// Keeps the shutdown channel open for as long as the app lives.
    _stop_tx: Arc<watch::Sender<bool>>,
    /// Digest of the last fingerprint, so a stream that reconnects can tell it missed something.
    version: AtomicU64,
    /// Wakes the refresher: a revision was queued, the index is needed, or a tab started.
    pub(crate) wake: Notify,
    /// Tells the warm-up that the index moved.
    refreshed: Notify,
    /// A page that needs the index was asked for, see `index.build_on_start`.
    demanded: AtomicBool,
    /// Set when the cache was deleted while serving: the loop rebuilds HEAD even if unmoved.
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
    /// Last time a page read the insights source, see [`App::release_idle`].
    records_used: Mutex<Instant>,
    /// Warm-up jobs running. Their reads are not a visitor's, so they do not keep the caches.
    warming: std::sync::atomic::AtomicUsize,
    /// Idle releases that freed something, served in `/api/progress` for tests to wait on.
    releases: AtomicU64,
    /// Ranges read from git for a compare, so a test can tell a memo hit from a read.
    pub(crate) git_ranges: AtomicU64,
    memo: Mutex<crate::bounded::Bounded<(Instant, Arc<String>)>>,
    dirty: Mutex<Option<(Instant, bool)>>,
    /// Open streams on a page that shows the working tree. Zero means no scan runs.
    pub(crate) wt_watchers: std::sync::atomic::AtomicUsize,
    /// Working tree scans finished so far, counted once the result is recorded: a test that
    /// sees the count move knows the baseline is in place, and can tell idle from busy.
    pub(crate) wt_scans: AtomicU64,
    /// The working tree signature of the oldest worktree page rendered since the last scan. A
    /// tab's first scan compares against it, so an edit between the render and that scan shows.
    pub(crate) wt_rendered: Mutex<Option<u64>>,
    /// Off ignores OS file events, so a test can prove the safety rescan alone finds a change.
    pub(crate) os_events: AtomicBool,
    /// This repository's `/api/v1` state, so a reload reaches its settings.
    api: std::sync::OnceLock<gitterrace_api::ApiState>,
    /// Every per-request cache, dropped together when the repository is idle.
    pub(crate) caches: crate::caches::Caches,
}

impl App {
    /// An app with the default settings.
    #[cfg(test)]
    pub(crate) fn new(repo: Repo, index: Index) -> Self {
        Self::configured(repo, index, &Env::new(Config::default()))
    }

    pub(crate) fn configured(repo: Repo, index: Index, env: &Env) -> Self {
        let name = repo.name();
        let live_streams = env.cfg().server.live_streams;
        let streams = Arc::new(Semaphore::new(live_streams));
        let cfg = env.cfg();
        Self {
            repo,
            index,
            env: env.clone(),
            streams,
            streams_cap: Mutex::new(live_streams),
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
            records_used: Mutex::new(Instant::now()),
            warming: std::sync::atomic::AtomicUsize::new(0),
            releases: AtomicU64::new(0),
            git_ranges: AtomicU64::new(0),
            memo: Mutex::new(crate::bounded::Bounded::new(cfg.cache_bytes(Cache::Pages))),
            dirty: Mutex::new(None),
            wt_watchers: std::sync::atomic::AtomicUsize::new(0),
            wt_scans: AtomicU64::new(0),
            wt_rendered: Mutex::new(None),
            os_events: AtomicBool::new(true),
            caches: crate::caches::Caches::new(&cfg),
            api: std::sync::OnceLock::new(),
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
        if self.public() || !crate::features::on(gitterrace_config::Feature::Actions) {
            return Vec::new();
        }
        self.env.lookups(sha)
    }

    /// Whether pages offer the settings page: its routes are mounted on this server.
    pub(crate) fn settings_on(&self) -> bool {
        self.env.settings_on()
    }

    pub(crate) fn actions_note(&self) -> Option<String> {
        if !crate::features::on(gitterrace_config::Feature::Actions) {
            return None;
        }
        self.env.actions_note()
    }

    /// Takes a reloaded config: limits, stream cap, rendered pages and open tabs.
    /// Hands the app its `/api/v1` state, once, so a reload reconfigures it too.
    /// Links this repository's `/api/v1` state. An API answer that meets an unreadable cache
    /// starts the same rebuild a page does ([`App::unreadable_now`]), one for both.
    pub(crate) fn set_api(self: &Arc<Self>, api: gitterrace_api::ApiState) {
        let app = Arc::downgrade(self);
        api.on_unreadable(Arc::new(move || {
            if let Some(a) = app.upgrade()
                && let Err(e) = a.unreadable_now()
            {
                tracing::warn!("rebuilding the unreadable cache for the API failed: {e}");
            }
        }));
        self.api.set(api).ok();
    }

    fn reconfigure(&self, cfg: &Config) {
        if let Some(api) = self.api.get() {
            api.reconfigure(cfg);
        }
        self.index.set_block_cache(cfg.cache_bytes(Cache::Blocks));
        let (text_bytes, blame_bytes) = repo_limits(cfg);
        self.repo.set_limits(text_bytes, blame_bytes);
        self.repo.set_worktree_limits(
            cfg.limits.worktree_files,
            (cfg.limits.worktree_diff_kb as u64) << 10,
        );
        resize(&self.streams, &self.streams_cap, cfg.server.live_streams);
        if let Ok(mut g) = self.memo.lock() {
            g.clear();
            g.set_budget(cfg.cache_bytes(Cache::Pages));
        }
        self.caches.set_budget(cfg);
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

    /// Size in bytes of the highlighted-code cache, its share of `resources.cache_mb`.
    pub(crate) fn code_cache_bytes(&self) -> usize {
        self.cfg().cache_bytes(Cache::Code)
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
        if let Ok(g) = self.caches.last_commits.lock()
            && let Some(v) = g.get(&key)
        {
            return Ok(v);
        }
        // A ready index names the commits that changed anything under `dir`; the walk visits
        // only those instead of every commit in history.
        let among = self.accelerate(commit, |s| touched::touched_names(&**s, dir))?;
        let (found, complete) =
            gitterrace_git::last_commits_among(&self.repo, commit, dir, among.as_deref(), budget)?;
        let v = Arc::new(found);
        if !complete {
            return Ok(v);
        }
        if let Ok(mut g) = self.caches.last_commits.lock() {
            g.put(key, Arc::clone(&v));
        }
        Ok(v)
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
    let cfg = app.cfg();
    let absolute = cfg.display.dates == gitterrace_types::DateStyle::Absolute;
    // Every page is built here, so every page reads one table of what is switched on.
    let on = crate::features::Effective::of(&cfg, Some(&app.name));
    tokio::task::spawn_blocking(move || {
        crate::features::with(on, || {
            crate::fmt::with_dates(absolute, || {
                crate::fmt::with_base(&app.base, || crate::fmt::with_rev(&rev, || f(&app)))
            })
        })
    })
    .await?
}

#[cfg(test)]
mod tests;
