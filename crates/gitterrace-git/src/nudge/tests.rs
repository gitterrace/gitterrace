use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn cfg(root: &Path, rescan: Duration, os_events: bool) -> NudgeConfig {
    NudgeConfig {
        roots: vec![root.canonicalize().expect("root")],
        skip: Arc::new(|p: &Path, _| p.file_name().is_some_and(|n| n == "target")),
        debounce: Duration::from_millis(150),
        max_delay: Duration::from_millis(600),
        rescan,
        max_watches: 1000,
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

/// How long a test waits for an event. Generous: a loaded build machine delays macOS file
/// events by seconds, and how fast a change shows is proven by the web tests.
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
    assert!(!relevant(
        &c,
        &ev(write, "target/a.o"),
        none,
        &mut 0,
        &|_| {}
    ));
    let none: Option<&mut notify::NullWatcher> = None;
    assert!(!relevant(&c, &ev(read, "src.rs"), none, &mut 0, &|_| {}));
    let none: Option<&mut notify::NullWatcher> = None;
    assert!(relevant(&c, &ev(write, "src.rs"), none, &mut 0, &|_| {}));
    let outside = notify::Event::new(write).add_path(PathBuf::from("/elsewhere/x"));
    let none: Option<&mut notify::NullWatcher> = None;
    assert!(!relevant(&c, &outside, none, &mut 0, &|_| {}));
}

/// A tree with more folders than the cap registers nothing past it and says so, so the
/// caller keeps no partial set of watches. Red when the count is not enforced.
#[test]
fn a_tree_above_the_watch_cap_is_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    for i in 0..6 {
        std::fs::create_dir(dir.path().join(format!("d{i}"))).expect("dir");
    }
    let c = cfg(dir.path(), Duration::from_secs(60), false);
    let mut w = notify::NullWatcher;
    let mut n = 0;
    let r = add_tree(&mut w, &c.roots[0], &c.skip, &mut n, 4);
    assert!(matches!(r, Err(NudgeError::TooMany(4))), "{r:?}");
    assert_eq!(n, 4);
    n = 0;
    add_tree(&mut w, &c.roots[0], &c.skip, &mut n, 7).expect("fits");
    assert_eq!(n, 7, "the root and six folders");
}

/// A directory created after start that would pass the cap gets no watch and is reported
/// once as degraded, so the safety timer covers it. Linux only: elsewhere one recursive
/// watch covers every folder and nothing is counted.
#[cfg(target_os = "linux")]
#[test]
fn a_folder_created_past_the_cap_degrades_instead_of_being_watched() {
    let dir = tempfile::tempdir().expect("tmp");
    let c = cfg(dir.path(), Duration::from_secs(60), false);
    let made = c.roots[0].join("new");
    std::fs::create_dir(&made).expect("dir");
    let e = notify::Event::new(EventKind::Create(notify::event::CreateKind::Folder)).add_path(made);
    let degraded = Arc::new(AtomicUsize::new(0));
    let d = Arc::clone(&degraded);
    let on = move |x: Nudge| {
        if x == Nudge::Degraded {
            d.fetch_add(1, Ordering::SeqCst);
        }
    };
    let mut w = notify::NullWatcher;
    let mut watched = c.max_watches;
    assert!(relevant(&c, &e, Some(&mut w), &mut watched, &on));
    assert_eq!(degraded.load(Ordering::SeqCst), 1, "reported once");
    assert_eq!(watched, c.max_watches, "no watch was added past the cap");
    let mut room = 0;
    assert!(relevant(&c, &e, Some(&mut w), &mut room, &on));
    assert_eq!(
        degraded.load(Ordering::SeqCst),
        1,
        "under the cap it is watched quietly"
    );
    assert_eq!(room, 1);
}

#[cfg(target_os = "linux")]
#[test]
fn above_the_cap_the_safety_timer_still_nudges_and_degrades() {
    let _one = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    for i in 0..4 {
        std::fs::create_dir(dir.path().join(format!("d{i}"))).expect("dir");
    }
    let degraded = Arc::new(AtomicUsize::new(0));
    let (n, count) = counting();
    let d = Arc::clone(&degraded);
    let on = move |x: Nudge| {
        if x == Nudge::Degraded {
            d.fetch_add(1, Ordering::SeqCst);
        }
        count(x);
    };
    let mut c = cfg(dir.path(), Duration::from_millis(200), true);
    c.max_watches = 2;
    let _w = Nudger::start(c, on).expect("starts");
    assert!(wait_for(&n, 3, Duration::from_secs(3)), "timer nudges");
    assert_eq!(degraded.load(Ordering::SeqCst), 1);
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
    let t = std::thread::spawn(move || run::<notify::NullWatcher>(&c, None, 0, &rx, &on_nudge));
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
