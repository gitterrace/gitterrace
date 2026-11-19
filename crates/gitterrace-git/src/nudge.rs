//! A payload-free "look again" signal from the operating system's file events.
//!
//! The signal never says what changed. The caller keeps its own source of truth (a scan) and
//! runs it when nudged. A burst of events becomes one nudge after a quiet window, and a safety
//! timer nudges anyway every `rescan`, so a lost event costs at most that long, never a missed
//! change. Overflow and rescan notices from the OS nudge at once.
//!
//! On Linux every watched directory costs one inotify watch, so directories are registered one
//! by one and the `skip` filter is asked before each: a skipped tree is never registered. Other
//! platforms watch each root recursively with one OS stream and apply `skip` to the events.
//!
//! Self-contained on purpose: no types from the rest of the crate, so it can move on its own.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use notify::{EventKind, RecursiveMode, Watcher};

/// Wall clock running ahead of the monotonic clock by more than this means the machine slept.
const SLEEP_GAP: Duration = Duration::from_secs(5);

/// Says whether a path (absolute, `is_dir` when known to be a directory) is of no interest.
pub type Skip = Arc<dyn Fn(&Path, bool) -> bool + Send + Sync>;

/// What to watch and how often to nudge.
#[derive(Clone)]
pub struct NudgeConfig {
    /// Directories watched with everything below them that `skip` lets through.
    pub roots: Vec<PathBuf>,
    pub skip: Skip,
    /// Quiet time after the last event before the nudge.
    pub debounce: Duration,
    /// Longest a steady stream of events may hold the nudge back.
    pub max_delay: Duration,
    /// The safety timer: a nudge at least this often, events or not.
    pub rescan: Duration,
    /// Most directories watched on Linux, where each costs an inotify watch shared with every
    /// other program of the user. Above it no watch is kept and only the safety timer nudges.
    pub max_watches: usize,
    /// Off starts no OS watcher, so only the safety timer nudges. For tests of that timer.
    pub os_events: bool,
}

/// Why the watcher could not start. The caller then polls.
#[derive(Debug, thiserror::Error)]
pub enum NudgeError {
    #[error("the system limit on file watches is reached (on Linux: fs.inotify.max_user_watches)")]
    Limit,
    /// The tree has more folders than `max_watches`.
    #[error("more folders than the {0} watches gitterrace may use")]
    TooMany(usize),
    #[error("file events unavailable: {0}")]
    Os(String),
}

/// What the callback is told. Neither carries a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    /// Something may have changed: look again.
    Look,
    /// The watch limit or the cap is reached, so some folders have no watch. Look again and
    /// poll from now on.
    Degraded,
}

enum Msg {
    Event(notify::Result<notify::Event>),
    Stop,
}

/// The running watcher. Its thread and OS watches end when this is dropped.
pub struct Nudger {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl Nudger {
    /// Starts watching. Registration happens before this returns, so an error here (the watch
    /// limit, a missing root) is the caller's cue to poll instead.
    pub fn start(
        cfg: NudgeConfig,
        on_nudge: impl Fn(Nudge) + Send + 'static,
    ) -> Result<Self, NudgeError> {
        let (tx, rx) = mpsc::channel();
        let mut watched = 0;
        let mut capped = false;
        let watch = if cfg.os_events {
            let events = tx.clone();
            let mut w = notify::recommended_watcher(move |e| {
                // An error only says the watcher thread already stopped.
                events.send(Msg::Event(e)).ok();
            })
            .map_err(|e| os_error(&e))?;
            match register(&mut w, &cfg, &mut watched) {
                Ok(()) => Some(w),
                // Too big a tree: drop every watch (a partial set would miss changes quietly)
                // and let the safety timer rescan.
                Err(NudgeError::TooMany(_)) => {
                    capped = true;
                    None
                }
                Err(e) => return Err(e),
            }
        } else {
            None
        };
        let thread = std::thread::Builder::new()
            .name("fs-nudge".into())
            .spawn(move || {
                if capped {
                    on_nudge(Nudge::Degraded);
                }
                run(&cfg, watch, watched, &rx, &on_nudge);
            })
            .map_err(|e| NudgeError::Os(e.to_string()))?;
        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }
}

impl Drop for Nudger {
    fn drop(&mut self) {
        // An error only says the thread already ended.
        self.tx.send(Msg::Stop).ok();
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

const fn per_directory() -> bool {
    cfg!(any(target_os = "linux", target_os = "android"))
}

fn os_error(e: &notify::Error) -> NudgeError {
    if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) {
        NudgeError::Limit
    } else {
        NudgeError::Os(e.to_string())
    }
}

fn register(
    w: &mut impl Watcher,
    cfg: &NudgeConfig,
    watched: &mut usize,
) -> Result<(), NudgeError> {
    for root in &cfg.roots {
        if per_directory() {
            add_tree(w, root, &cfg.skip, watched, cfg.max_watches)?;
        } else {
            w.watch(root, RecursiveMode::Recursive)
                .map_err(|e| os_error(&e))?;
        }
    }
    Ok(())
}

/// Registers `dir` and every directory below it that `skip` lets through, one watch each.
/// `watched` counts the watches in force; reaching `max` is [`NudgeError::TooMany`].
fn add_tree(
    w: &mut impl Watcher,
    dir: &Path,
    skip: &Skip,
    watched: &mut usize,
    max: usize,
) -> Result<(), NudgeError> {
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        if *watched >= max {
            return Err(NudgeError::TooMany(max));
        }
        match w.watch(&d, RecursiveMode::NonRecursive) {
            Ok(()) => *watched += 1,
            Err(e) if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) => {
                return Err(NudgeError::Limit);
            }
            // A directory that vanished or cannot be read meanwhile has nothing to watch.
            Err(_) => continue,
        }
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            // Symlinks are not followed: a link to a big tree outside must not cost watches.
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                let p = e.path();
                if !skip(&p, true) {
                    todo.push(p);
                }
            }
        }
    }
    Ok(())
}

/// The debounce loop. Owns the OS watcher so new directories can be registered as they appear.
fn run<W: Watcher>(
    cfg: &NudgeConfig,
    watch: Option<W>,
    watched: usize,
    rx: &Receiver<Msg>,
    on_nudge: &impl Fn(Nudge),
) {
    run_decided(cfg, watch, watched, rx, &|n, _| on_nudge(n));
}

/// [`run`], telling the callback when each nudge was decided. A test reads the spacing from
/// these instants: a callback stamping its own clock measures thread scheduling, not the rule.
fn run_decided<W: Watcher>(
    cfg: &NudgeConfig,
    mut watch: Option<W>,
    mut watched: usize,
    rx: &Receiver<Msg>,
    on_decided: &impl Fn(Nudge, Instant),
) {
    let on_nudge = &|n: Nudge| on_decided(n, Instant::now());
    let mut first: Option<Instant> = None;
    let mut last = Instant::now();
    let mut rescan_at = Instant::now() + cfg.rescan;
    let mut clocks = (Instant::now(), wall_clock());
    // An overloaded OS reports drops in a storm, one notice per delivered batch. Each asks to
    // look now, but only the first is acted on at once: later ones wait until `max_delay`
    // passed since the last nudge, so a storm costs one scan per `max_delay`, not per notice.
    let mut nudged: Option<Instant> = None;
    let mut owed = false;
    loop {
        let now = Instant::now();
        let mut deadline = first.map_or(rescan_at, |f| {
            (last + cfg.debounce).min(f + cfg.max_delay).min(rescan_at)
        });
        if owed && let Some(t) = nudged {
            deadline = deadline.min(t + cfg.max_delay);
        }
        let msg = rx.recv_timeout(deadline.saturating_duration_since(now));
        let mut urgent = slept(&mut clocks);
        match msg {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Msg::Event(Ok(e))) => {
                if e.need_rescan() {
                    urgent = true;
                } else if relevant(cfg, &e, watch.as_mut(), &mut watched, on_nudge) {
                    last = Instant::now();
                    first.get_or_insert(last);
                }
            }
            // An overflow or a backend error: events may be lost, so look now.
            Ok(Msg::Event(Err(e))) => {
                if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) {
                    watch = None;
                    on_nudge(Nudge::Degraded);
                }
                urgent = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        owed |= urgent;
        let urgent_due = owed && nudged.is_none_or(|t| now >= t + cfg.max_delay);
        let settled = first.is_some_and(|f| now >= last + cfg.debounce || now >= f + cfg.max_delay);
        if urgent_due || settled || now >= rescan_at {
            first = None;
            owed = false;
            nudged = Some(now);
            rescan_at = now + cfg.rescan;
            on_decided(Nudge::Look, now);
        }
    }
}

/// True when the wall clock jumped ahead of the monotonic one since the last call: a sleep
/// and wake, during which events may not have been delivered.
fn slept(clocks: &mut (Instant, SystemTime)) -> bool {
    let (mono, wall) = (Instant::now(), wall_clock());
    let mono_gap = mono.duration_since(clocks.0);
    let wall_gap = wall.duration_since(clocks.1).unwrap_or_default();
    *clocks = (mono, wall);
    wall_gap > mono_gap + SLEEP_GAP
}

/// The real wall clock, never a frozen test clock: only its jumps against the monotonic clock
/// matter here, and no time is shown.
fn wall_clock() -> SystemTime {
    SystemTime::now()
}

/// Whether an event touches a path of interest. Pure reads (opens, closes) never count. On
/// Linux a new directory is registered here, before its contents can be missed.
fn relevant<W: Watcher>(
    cfg: &NudgeConfig,
    e: &notify::Event,
    watch: Option<&mut W>,
    watched: &mut usize,
    on_nudge: &impl Fn(Nudge),
) -> bool {
    if matches!(e.kind, EventKind::Access(_)) {
        return false;
    }
    let mut hit = false;
    let mut watch = watch;
    for p in &e.paths {
        let is_dir = std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
        if skipped(cfg, p) {
            continue;
        }
        hit = true;
        if per_directory()
            && is_dir
            && matches!(e.kind, EventKind::Create(_) | EventKind::Modify(_))
            && let Some(w) = watch.as_deref_mut()
            && matches!(
                add_tree(w, p, &cfg.skip, watched, cfg.max_watches),
                Err(NudgeError::Limit | NudgeError::TooMany(_))
            )
        {
            on_nudge(Nudge::Degraded);
        }
    }
    hit
}

/// True when the path or a directory between it and its root is skipped.
fn skipped(cfg: &NudgeConfig, p: &Path) -> bool {
    let Some(root) = cfg.roots.iter().find(|r| p.starts_with(r)) else {
        return true;
    };
    let mut at = p;
    let mut is_dir = std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
    while at != root.as_path() {
        if (cfg.skip)(at, is_dir) {
            return true;
        }
        is_dir = true;
        match at.parent() {
            Some(up) => at = up,
            None => break,
        }
    }
    false
}

#[cfg(test)]
mod tests;
