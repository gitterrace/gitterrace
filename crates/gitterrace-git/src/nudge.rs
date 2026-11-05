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
    /// Off starts no OS watcher, so only the safety timer nudges. For tests of that timer.
    pub os_events: bool,
}

/// Why the watcher could not start. The caller then polls.
#[derive(Debug, thiserror::Error)]
pub enum NudgeError {
    #[error("the system limit on file watches is reached (on Linux: fs.inotify.max_user_watches)")]
    Limit,
    #[error("file events unavailable: {0}")]
    Os(String),
}

/// What the callback is told. Neither carries a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    /// Something may have changed: look again.
    Look,
    /// The watch limit is reached while running, so some folders have no watch. Look again and
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
        let watch = if cfg.os_events {
            let events = tx.clone();
            let mut w = notify::recommended_watcher(move |e| {
                // An error only says the watcher thread already stopped.
                events.send(Msg::Event(e)).ok();
            })
            .map_err(|e| os_error(&e))?;
            register(&mut w, &cfg)?;
            Some(w)
        } else {
            None
        };
        let thread = std::thread::Builder::new()
            .name("fs-nudge".into())
            .spawn(move || run(&cfg, watch, &rx, &on_nudge))
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

fn register(w: &mut impl Watcher, cfg: &NudgeConfig) -> Result<(), NudgeError> {
    for root in &cfg.roots {
        if per_directory() {
            add_tree(w, root, &cfg.skip)?;
        } else {
            w.watch(root, RecursiveMode::Recursive)
                .map_err(|e| os_error(&e))?;
        }
    }
    Ok(())
}

/// Registers `dir` and every directory below it that `skip` lets through, one watch each.
fn add_tree(w: &mut impl Watcher, dir: &Path, skip: &Skip) -> Result<(), NudgeError> {
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        match w.watch(&d, RecursiveMode::NonRecursive) {
            Ok(()) => {}
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
    mut watch: Option<W>,
    rx: &Receiver<Msg>,
    on_nudge: &impl Fn(Nudge),
) {
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
                } else if relevant(cfg, &e, watch.as_mut(), on_nudge) {
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
            on_nudge(Nudge::Look);
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
            && matches!(add_tree(w, p, &cfg.skip), Err(NudgeError::Limit))
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
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn cfg(root: &Path, rescan: Duration, os_events: bool) -> NudgeConfig {
        NudgeConfig {
            roots: vec![root.canonicalize().expect("root")],
            skip: Arc::new(|p: &Path, _| p.file_name().is_some_and(|n| n == "target")),
            debounce: Duration::from_millis(150),
            max_delay: Duration::from_millis(600),
            rescan,
            os_events,
        }
    }

    fn counting() -> (Arc<AtomicUsize>, impl Fn(Nudge) + Send + 'static) {
        let n = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&n);
        (n, move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        })
    }

    /// How long a test waits for an event. Generous: a loaded build machine delays macOS file events by
    /// seconds, and how fast a change shows is proven by the web tests.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// These tests time OS events, so they run one at a time.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// macOS arms its event stream asynchronously and may still report the temp folder's own
    /// creation, so a test counts only once a second passed without a nudge.
    fn settle(n: &AtomicUsize) -> usize {
        let mut last = usize::MAX;
        for _ in 0..10 {
            let now = n.load(Ordering::SeqCst);
            if now == last {
                break;
            }
            last = now;
            std::thread::sleep(Duration::from_secs(1));
        }
        last
    }

    fn wait_for(n: &AtomicUsize, at_least: usize, within: Duration) -> bool {
        let end = Instant::now() + within;
        while Instant::now() < end {
            if n.load(Ordering::SeqCst) >= at_least {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Repeats `act` each second until the count reaches `at_least`: a loaded machine may drop
    /// or delay one event, and these tests prove an event arrives, not how fast.
    fn again(n: &AtomicUsize, at_least: usize, act: impl Fn()) -> bool {
        let end = Instant::now() + PATIENCE;
        while Instant::now() < end {
            act();
            if wait_for(n, at_least, Duration::from_secs(1)) {
                return true;
            }
        }
        false
    }

    #[test]
    fn a_write_nudges() {
        let _one = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tmp");
        let (n, f) = counting();
        let _w = Nudger::start(cfg(dir.path(), Duration::from_secs(60), true), f).expect("w");
        // Silence is proven on a real repository in the web tests, over a longer window.
        let base = settle(&n);
        let write = || std::fs::write(dir.path().join("a.txt"), "x").expect("write");
        assert!(again(&n, base + 1, write), "write seen");
    }

    #[test]
    fn a_file_saved_by_rename_and_a_new_directory_are_seen() {
        let _one = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tmp");
        let (n, f) = counting();
        let _w = Nudger::start(cfg(dir.path(), Duration::from_secs(60), true), f).expect("w");
        let base = settle(&n);
        std::fs::create_dir(dir.path().join("sub")).expect("dir");
        // On Linux the new folder must be registered before its file lands, or this is missed.
        let save = || {
            std::fs::write(dir.path().join("sub/.f.swp"), "x").expect("write");
            std::fs::rename(dir.path().join("sub/.f.swp"), dir.path().join("sub/f")).expect("mv");
        };
        assert!(again(&n, base + 1, save), "file in new dir");
    }

    #[test]
    fn a_skipped_tree_and_pure_reads_never_count() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::create_dir(dir.path().join("target")).expect("dir");
        let c = cfg(dir.path(), Duration::from_secs(60), false);
        let root = &c.roots[0];
        let ev = |kind, p: &str| notify::Event::new(kind).add_path(root.join(p));
        let none: Option<&mut notify::NullWatcher> = None;
        let write = EventKind::Modify(notify::event::ModifyKind::Any);
        let read = EventKind::Access(notify::event::AccessKind::Any);
        assert!(!relevant(&c, &ev(write, "target/a.o"), none, &|_| {}));
        let none: Option<&mut notify::NullWatcher> = None;
        assert!(!relevant(&c, &ev(read, "src.rs"), none, &|_| {}));
        let none: Option<&mut notify::NullWatcher> = None;
        assert!(relevant(&c, &ev(write, "src.rs"), none, &|_| {}));
        let outside = notify::Event::new(write).add_path(PathBuf::from("/elsewhere/x"));
        let none: Option<&mut notify::NullWatcher> = None;
        assert!(!relevant(&c, &outside, none, &|_| {}));
    }

    #[test]
    fn a_burst_of_writes_is_one_or_a_few_nudges() {
        let _one = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tmp");
        let (n, f) = counting();
        let _w = Nudger::start(cfg(dir.path(), Duration::from_secs(60), true), f).expect("w");
        let base = settle(&n);
        let t0 = Instant::now();
        // A stream that macOS starts late sees nothing of the first pass, so the burst is
        // written again until one is seen, like the other tests that wait for an event.
        let burst = || {
            for i in 0..5000 {
                std::fs::write(dir.path().join(format!("f{i}")), "x").expect("write");
            }
        };
        assert!(again(&n, base + 1, burst), "burst seen");
        let got = settle(&n) - base;
        // `settle` returns a second after the last nudge, so this is how long nudges kept
        // coming. At most one per `max_delay` (600 ms) while events arrive, plus the tail and an
        // overflow notice; a loaded machine delivering late earns a longer window, but one nudge
        // per file (thousands) never fits it.
        let busy = t0.elapsed().saturating_sub(Duration::from_secs(1));
        let most = usize::try_from(busy.as_millis() / 600).unwrap_or(usize::MAX) + 6;
        assert!(
            got <= most,
            "{got} nudges for 5000 files over {busy:?}, most {most}"
        );
    }

    #[test]
    fn a_storm_of_drop_notices_nudges_at_most_once_per_max_delay() {
        // Fed by hand, no OS stream: an overloaded machine sends a drop notice with every
        // batch, and each one nudging at once was a scan storm. The gap between nudges is
        // read from their own timestamps, so machine speed cannot change the verdict.
        let dir = tempfile::tempdir().expect("tmp");
        let c = cfg(dir.path(), Duration::from_secs(60), false);
        let max_delay = c.max_delay;
        let (tx, rx) = mpsc::channel();
        let stamps = Arc::new(std::sync::Mutex::new(Vec::<Instant>::new()));
        let sink = Arc::clone(&stamps);
        let on_nudge = move |_| {
            sink.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Instant::now());
        };
        let t = std::thread::spawn(move || run::<notify::NullWatcher>(&c, None, &rx, &on_nudge));
        let rescan = || notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
        let storm_end = Instant::now() + max_delay * 3;
        let mut sent = 0;
        while Instant::now() < storm_end {
            tx.send(Msg::Event(Ok(rescan()))).expect("send");
            sent += 1;
            std::thread::sleep(Duration::from_millis(5));
        }
        // A notice after the first is still answered, once its `max_delay` is up.
        std::thread::sleep(max_delay * 2);
        tx.send(Msg::Stop).expect("stop");
        t.join().expect("join");
        let at = stamps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert!(
            at.len() >= 2,
            "the first nudge is at once and the owed one follows: {at:?}"
        );
        assert!(at.len() < sent, "{} nudges for {sent} notices", at.len());
        let tightest = at.windows(2).map(|w| w[1] - w[0]).min().expect("a gap");
        assert!(
            tightest >= max_delay,
            "nudges came {tightest:?} apart, less than the {max_delay:?} between them"
        );
    }

    #[test]
    fn without_os_events_the_safety_timer_still_nudges() {
        let _one = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tmp");
        let (n, f) = counting();
        let _w = Nudger::start(cfg(dir.path(), Duration::from_millis(300), false), f).expect("w");
        assert!(wait_for(&n, 2, Duration::from_millis(1500)), "timer nudges");
    }

    #[test]
    fn dropping_the_watcher_ends_its_thread() {
        let _one = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tmp");
        let (n, f) = counting();
        let w = Nudger::start(cfg(dir.path(), Duration::from_millis(50), true), f).expect("w");
        assert_eq!(Arc::strong_count(&n), 2, "the thread holds the callback");
        drop(w);
        assert_eq!(
            Arc::strong_count(&n),
            1,
            "the callback is gone with the thread"
        );
    }
}
