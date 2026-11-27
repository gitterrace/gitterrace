//! Shared state and the background index refresher.

use std::{
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gitterrace_config::{Cache, Config};
#[cfg(test)]
use gitterrace_git::LastCommit;
use gitterrace_git::Repo;
use gitterrace_index::{Index, Progress};
use tokio::sync::{Notify, broadcast, watch};

use crate::live::Change;

/// Rendered stats pages are reused this long; time-relative windows move slowly.
const MEMO_TTL: Duration = Duration::from_secs(60);
/// How long a dirty-worktree answer is reused, so every page need not run a status.
pub(crate) const DIRTY_TTL: Duration = Duration::from_secs(2);
/// The source, its block cache and the tables built over it are dropped after this long without
/// a page that reads them, so an idle server keeps none of them; a page after the release opens
/// the set again. The page memo still answers the unfiltered pages in between.
pub(crate) const RECORDS_IDLE: Duration = Duration::from_secs(30);

mod child;
mod dirty;
mod events;
mod history;
mod jobs;
mod memo;
mod records;
mod refresh;
mod sched;
mod state;
mod touched;

pub(crate) use dirty::DirtyMemo;

pub use child::index_child;
pub(crate) use gitterrace_api::{Slot, Slots};
pub(crate) use history::PathLog;
pub(crate) use jobs::{blocking, cancel_flag};
pub(crate) use refresh::{keep_index_fresh_supervised, release_when_idle};
pub(crate) use state::RepoState;

/// What every repository of one server shares: settings, job cap and the shutdown signal.
#[derive(Clone)]
pub(crate) struct Env {
    cfg: Arc<RwLock<Arc<Config>>>,
    /// Every repository app built so far, so a reload reaches them.
    apps: Arc<Mutex<Vec<Weak<App>>>>,
    jobs: Slots,
    stop: Arc<watch::Sender<bool>>,
    /// Why `actions.toml` is not in force; rendered into every page so the note is there
    /// at first paint instead of arriving with a poll and pushing the page down.
    actions_note: Arc<RwLock<Option<Arc<crate::actions::Status>>>>,
    /// The `actions.toml` Send to targets, reloaded with the file, read by the commit page.
    sends: Arc<RwLock<Option<SharedActions>>>,
    /// The owner proof every write checks, set when the server is built (see `owner.rs`).
    pub(crate) owner: Arc<RwLock<Option<Arc<crate::owner::Owner>>>>,
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
    /// Repository-list notices for the open home pages (`GET /events` beside `/`).
    pub(crate) home: broadcast::Sender<Arc<Change>>,
    /// Counts the list changes, so a home page that reconnects can tell it missed one.
    home_v: Arc<AtomicU64>,
    /// Open home streams, capped by `server.live_streams` as each repository's are.
    pub(crate) home_streams: Slots,
    /// The served repositories, for the log's related links. Empty for a lone repository.
    pub(crate) related: crate::related::Peers,
    /// MiB of raw file bytes in memory at once, `limits.raw_concurrent_mb`: a `/raw` request
    /// holds its share until the body is sent.
    pub(crate) raw_budget: Slots,
    /// The worker budget of the index builds, from the machine's load.
    sched: Arc<sched::Scheduler>,
}

/// The live `actions.toml`, swapped whole on reload.
type SharedActions = Arc<RwLock<Arc<gitterrace_actions::Config>>>;

/// Page-building slots for `cfg`: 0 means one per core.
pub(crate) fn job_slots(cfg: &Config) -> usize {
    match cfg.resources.blocking_jobs {
        0 => std::thread::available_parallelism().map_or(2, usize::from),
        n => n,
    }
}

/// `limits.raw_concurrent_mb` as MiB permits; the config bounds it below the semaphore's cap.
fn raw_slots(cfg: &Config) -> usize {
    usize::try_from(cfg.limits.raw_concurrent_mb).unwrap_or(1)
}

/// Text cap and blame cache size in bytes for a repository handle.
pub(crate) fn repo_limits(cfg: &Config) -> (u64, usize) {
    (cfg.limits.text_kb << 10, cfg.cache_bytes(Cache::Blame))
}

impl Env {
    /// One page-building slot, for blocking work outside [`blocking`] such as the
    /// multi-repository home. Move it into the blocking task, so a dropped request frees it
    /// only when the work exits.
    pub(crate) async fn job_slot(&self) -> std::io::Result<Slot> {
        self.jobs.take().await.map_err(|_| std::io::Error::other("blocking job slots closed"))
    }

    pub(crate) fn new(cfg: Config) -> Self {
        let jobs = job_slots(&cfg);
        let public = cfg.server.is_public;
        let live_streams = cfg.server.live_streams;
        let raw = raw_slots(&cfg);
        Self {
            public,
            cfg: Arc::new(RwLock::new(Arc::new(cfg))),
            apps: Arc::new(Mutex::new(Vec::new())),
            jobs: Slots::new(jobs),
            stop: Arc::new(watch::channel(false).0),
            actions_note: Arc::default(),
            sends: Arc::default(),
            owner: Arc::default(),
            changes: None,
            settings_dir: None,
            search: Vec::new(),
            settings_on: Arc::default(),
            home: broadcast::channel(16).0,
            home_v: Arc::default(),
            home_streams: Slots::new(live_streams),
            related: crate::related::Peers::default(),
            raw_budget: Slots::new(raw),
            sched: sched::Scheduler::system(),
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

    /// Hands the owner proof to every repository this server serves, so a page write can check it.
    pub(crate) fn set_owner(&self, owner: &Arc<crate::owner::Owner>) {
        if let Ok(mut g) = self.owner.write() {
            *g = Some(Arc::clone(owner));
        }
    }

    /// The owner proof writes check, `None` when none was set (an export, a bare router).
    pub(crate) fn owner(&self) -> Option<Arc<crate::owner::Owner>> {
        self.owner.read().ok().and_then(|g| g.as_ref().map(Arc::clone))
    }

    /// `(label, url)` for each Send to target with a `lookup`, filled for `sha`.
    pub(crate) fn lookups(&self, sha: &str) -> Vec<(String, String)> {
        let Some(config) = self.sends.read().ok().and_then(|g| g.as_ref().map(Arc::clone)) else {
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

    /// Every live app built on this environment.
    pub(crate) fn apps(&self) -> Vec<Arc<App>> {
        self.apps.lock().map(|g| g.iter().filter_map(Weak::upgrade).collect()).unwrap_or_default()
    }

    /// Tells every open tab that the set of served repositories changed, so the switcher shows
    /// a repository that joined without a reload and an open home lists it.
    pub(crate) fn repos_changed(&self) {
        let v = self.home_v.fetch_add(1, Ordering::AcqRel) + 1;
        // An error only says no home page is open.
        self.home.send(Arc::new(Change::repos(v))).ok();
        for app in self.apps() {
            // A log page links the related repositories: a joined or gone one changes it.
            if let Ok(mut g) = app.memo.lock() {
                g.clear();
            }
            // An error only says no tab is open.
            app.changes.send(Arc::new(Change::repos(app.version()))).ok();
        }
    }

    /// Turns true at shutdown: ends the home page streams.
    pub(crate) fn stopping(&self) -> watch::Receiver<bool> {
        self.stop.subscribe()
    }

    /// How many times the repository list changed, rendered into the home page.
    pub(crate) fn home_version(&self) -> u64 {
        self.home_v.load(Ordering::Acquire)
    }

    /// The settings in force now.
    pub(crate) fn cfg(&self) -> Arc<Config> {
        self.cfg.read().map_or_else(|e| Arc::clone(&e.into_inner()), |g| Arc::clone(&g))
    }

    /// How many repositories are served: more than one shows the switcher from the first paint.
    pub(crate) fn served(&self) -> usize {
        self.apps.lock().map_or(1, |g| g.iter().filter(|w| w.strong_count() > 0).count())
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
        let api_on = |c: &Config| c.feature_on(gitterrace_config::Feature::Api, None);
        if self.public && api_on(&new) && !api_on(&old) {
            tracing::warn!("features.api_enabled turned on: this public server now serves /api/v1");
        }
        let new = Arc::new(new);
        match self.cfg.write() {
            Ok(mut g) => *g = Arc::clone(&new),
            Err(e) => *e.into_inner() = Arc::clone(&new),
        }
        self.jobs.resize(job_slots(&new));
        self.home_streams.resize(new.server.live_streams);
        self.raw_budget.resize(raw_slots(&new));
        for app in self.apps() {
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
    pub(crate) streams: Slots,
    /// Change notices for the open streams. Nothing is sent while nobody listens.
    pub(crate) changes: broadcast::Sender<Arc<Change>>,
    pub(crate) stop: watch::Receiver<bool>,
    /// Keeps the shutdown channel open for as long as the app lives.
    _stop_tx: Arc<watch::Sender<bool>>,
    /// Digest of the last fingerprint, so a stream that reconnects can tell it missed something.
    version: AtomicU64,
    /// Wakes the refresher: a revision was queued, the index is needed, or a tab started.
    pub(crate) wake: Notify,
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
    pub(crate) progress: Mutex<Progress>,
    /// The index child running now, so `App::halt` (child.rs) can end it.
    pub(crate) index_child: Mutex<Option<std::process::Child>>,
    /// Held for a whole child update, so at most one child per repository runs (child.rs).
    pub(crate) index_run: Mutex<()>,
    /// Queued, indexing, warming, ready or failed, see `state.rs`.
    state: state::StateCell,
    /// Last time a page read the insights source, see [`App::release_idle`].
    records_used: Mutex<Instant>,
    /// Warm-up jobs running. Their reads are not a visitor's, so they do not keep the caches.
    warming: std::sync::atomic::AtomicUsize,
    /// Idle releases that freed something, served in `/api/progress` for tests to wait on.
    releases: AtomicU64,
    /// Ranges read from git for a compare, so a test can tell a memo hit from a read.
    pub(crate) git_ranges: AtomicU64,
    memo: Mutex<crate::bounded::Bounded<(Instant, Arc<memo::Packed>)>>,
    /// One lock per memo key being rendered, see [`App::render_lock`].
    pub(crate) rendering: Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    dirty: DirtyMemo,
    /// Open streams on a page that shows the working tree. Zero means no scan runs.
    pub(crate) wt_watchers: std::sync::atomic::AtomicUsize,
    /// Working tree scans finished so far, counted once the result is recorded: a test that
    /// sees the count move knows the baseline is in place, and can tell idle from busy.
    pub(crate) wt_scans: AtomicU64,
    /// Insights pages rendered rather than served from the memo, for the warm-up's guard.
    #[cfg(test)]
    pub(crate) renders: AtomicU64,
    /// Languages chain walks this app ran: the tests count work per page with it.
    pub(crate) chain_walks: AtomicU64,
    /// The part of `wt_scans` run on the poll timer, while the tree watch was still starting or
    /// had degraded, so a test can tell scans that events caused from the documented one-second
    /// poll.
    pub(crate) wt_polled: AtomicU64,
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
        let streams = Slots::new(env.cfg().server.live_streams);
        let cfg = env.cfg();
        index.set_message_index(cfg.index.has_messages);
        Self {
            repo,
            index,
            env: env.clone(),
            streams,
            changes: broadcast::channel(16).0,
            stop: env.stop.subscribe(),
            _stop_tx: Arc::clone(&env.stop),
            version: AtomicU64::new(0),
            wake: Notify::new(),
            demanded: AtomicBool::new(false),
            rebuild: AtomicBool::new(false),
            key: name.clone(),
            name,
            base: String::new(),
            actions: None,
            progress: Mutex::new(Progress::default()),
            index_child: Mutex::new(None),
            index_run: Mutex::new(()),
            state: state::StateCell::default(),
            records_used: Mutex::new(Instant::now()),
            warming: std::sync::atomic::AtomicUsize::new(0),
            releases: AtomicU64::new(0),
            git_ranges: AtomicU64::new(0),
            memo: Mutex::new(crate::bounded::Bounded::new(cfg.cache_bytes(Cache::Pages))),
            rendering: Mutex::new(std::collections::HashMap::new()),
            dirty: DirtyMemo::default(),
            wt_watchers: std::sync::atomic::AtomicUsize::new(0),
            wt_scans: AtomicU64::new(0),
            #[cfg(test)]
            renders: AtomicU64::new(0),
            chain_walks: AtomicU64::new(0),
            wt_polled: AtomicU64::new(0),
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

    /// Links this repository's `/api/v1` state, once, so a reload reconfigures it too. An API
    /// answer that meets an unreadable cache starts the same rebuild a page does
    /// ([`App::unreadable_now`]), and an API answer over the pages' tip reads their tables.
    pub(crate) fn set_api(self: &Arc<Self>, api: gitterrace_api::ApiState) {
        let app = Arc::downgrade(self);
        api.on_unreadable(Arc::new(move || {
            if let Some(a) = app.upgrade()
                && let Err(e) = a.unreadable_now()
            {
                tracing::warn!("rebuilding the unreadable cache for the API failed: {e}");
            }
        }));
        api.share_pages(self.pages_share());
        api.share_jobs(self.env.jobs.clone());
        self.api.set(api).ok();
    }

    /// This repository's `/api/v1` state, once linked.
    pub(crate) fn api(&self) -> Option<&gitterrace_api::ApiState> {
        self.api.get()
    }

    fn reconfigure(&self, cfg: &Config) {
        if let Some(api) = self.api.get() {
            api.reconfigure(cfg);
        }
        self.index.set_block_cache(cfg.cache_bytes(Cache::Blocks));
        if self.index.message_index() != cfg.index.has_messages {
            self.index.set_message_index(cfg.index.has_messages);
            // HEAD did not move: the loop builds or drops the column only when asked to.
            self.rebuild.store(true, Ordering::Release);
        }
        let (text_bytes, blame_bytes) = repo_limits(cfg);
        self.repo.set_limits(text_bytes, blame_bytes);
        self.repo.set_worktree_limits(
            cfg.limits.worktree_files,
            (cfg.limits.worktree_diff_kb as u64) << 10,
        );
        self.streams.resize(cfg.server.live_streams);
        if let Ok(mut g) = self.memo.lock() {
            g.clear();
            g.set_budget(cfg.cache_bytes(Cache::Pages));
        }
        self.caches.set_budget(cfg);
        // The watcher loop re-reads `watch.*` when woken.
        self.wake.notify_one();
        // An error only says no tab is open.
        self.changes.send(Arc::new(Change::config(self.version()))).ok();
    }

    /// Whether the worktree differs from HEAD, reused for a couple of seconds.
    pub(crate) fn dirty(&self) -> bool {
        if self.public() {
            return false;
        }
        if let Some(v) = self.dirty.get() {
            return v;
        }
        let v = gitterrace_git::is_dirty(&self.repo);
        self.dirty.put(v);
        v
    }

    /// Records a freshly measured answer so the next event carries it.
    fn set_dirty(&self, v: bool) {
        self.dirty.put(v);
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
        if moved("head") || moved("refs") || moved("index") || moved("worktree") {
            self.dirty.clear();
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

    pub(crate) fn progress(&self) -> Progress {
        self.progress.lock().map(|g| *g).unwrap_or_default()
    }
}

#[cfg(test)]
mod env_tests;
#[cfg(test)]
mod tests;
