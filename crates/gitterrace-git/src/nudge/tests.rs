use std::sync::atomic::{AtomicUsize, Ordering};

use gitterrace_testkit::wait::pause_blocking;

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

/// These tests time OS events, so they run one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn wait_for(n: &AtomicUsize, at_least: usize, within: Duration) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if n.load(Ordering::SeqCst) >= at_least {
            return true;
        }
        pause_blocking(Duration::from_millis(20));
    }
    false
}

/// The documented promise: a change is seen within the rescan bound, whatever the OS stream
/// does. The test uses a short rescan and never times the OS events themselves.
#[test]
fn a_write_nudges() {
    let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    let (n, f) = counting();
    let rescan = Duration::from_millis(300);
    let _w = Nudger::start(cfg(dir.path(), rescan, true), f).expect("w");
    let before = n.load(Ordering::SeqCst);
    std::fs::write(dir.path().join("a.txt"), "x").expect("write");
    // One rescan period plus generous slack for a loaded machine.
    assert!(
        wait_for(&n, before + 1, rescan + Duration::from_secs(5)),
        "write not seen within the rescan bound"
    );
}

/// The documented promise: a change is seen within the rescan bound, whatever the OS stream
/// does. The test uses a short rescan and never times the OS events themselves.
#[test]
fn a_file_saved_by_rename_and_a_new_directory_are_seen() {
    let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    let (n, f) = counting();
    let rescan = Duration::from_millis(300);
    let _w = Nudger::start(cfg(dir.path(), rescan, true), f).expect("w");
    let before = n.load(Ordering::SeqCst);
    std::fs::create_dir(dir.path().join("sub")).expect("dir");
    std::fs::write(dir.path().join("sub/.f.swp"), "x").expect("write");
    std::fs::rename(dir.path().join("sub/.f.swp"), dir.path().join("sub/f")).expect("mv");
    // One rescan period plus generous slack for a loaded machine.
    assert!(
        wait_for(&n, before + 1, rescan + Duration::from_secs(5)),
        "change not seen within the rescan bound"
    );
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
    assert!(!relevant(&c, &ev(write, "target/a.o"), none, &mut 0, &|_| {}));
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
    assert_eq!(degraded.load(Ordering::SeqCst), 1, "under the cap it is watched quietly");
    assert_eq!(room, 1);
}

#[cfg(target_os = "linux")]
#[test]
fn above_the_cap_the_safety_timer_still_nudges_and_degrades() {
    let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let rescan = Duration::from_millis(200);
    let mut c = cfg(dir.path(), rescan, true);
    c.max_watches = 2;
    let _w = Nudger::start(c, on).expect("starts");
    assert!(wait_for(&n, 3, rescan * 3 + Duration::from_secs(3)), "timer nudges");
    assert_eq!(degraded.load(Ordering::SeqCst), 1);
}

/// Coalescing is measured from events fed by hand, because a real stream may deliver late or
/// drop events under load and the rescan timer adds nudges of its own. With the rescan far off,
/// every nudge here comes from the debounce, which allows at most one per `max_delay` while
/// events keep arriving, plus the tail after the last one.
#[test]
fn a_burst_of_writes_is_one_or_a_few_nudges() {
    let dir = tempfile::tempdir().expect("tmp");
    let conf = cfg(dir.path(), Duration::from_secs(60), false);
    let (root, debounce, max_delay) = (conf.roots[0].clone(), conf.debounce, conf.max_delay);
    let (nudges, on_nudge) = counting();
    let (tx, rx) = mpsc::channel();
    let worker =
        std::thread::spawn(move || run::<notify::NullWatcher>(&conf, None, 0, &rx, &on_nudge));
    let write = EventKind::Modify(notify::event::ModifyKind::Any);
    let t0 = Instant::now();
    for i in 0..5000 {
        let event = notify::Event::new(write).add_path(root.join(format!("f{i}")));
        tx.send(Msg::Event(Ok(event))).expect("send");
        if i % 500 == 0 {
            pause_blocking(Duration::from_millis(20));
        }
    }
    let streamed = t0.elapsed();
    // Let the debounce settle, then stop.
    pause_blocking(debounce + max_delay);
    tx.send(Msg::Stop).expect("stop");
    worker.join().expect("join");
    let got = nudges.load(Ordering::SeqCst);
    let most =
        usize::try_from(streamed.as_millis() / max_delay.as_millis()).unwrap_or(usize::MAX) + 2;
    assert!(got >= 1, "the burst was never answered");
    assert!(got <= most, "{got} nudges for 5000 events over {streamed:?}, most {most}");
}

#[test]
fn a_storm_of_drop_notices_nudges_at_most_once_per_max_delay() {
    // Fed by hand, no OS stream: an overloaded machine sends a drop notice with every
    // batch, and each one nudging at once was a scan storm. The gap between nudges is read
    // from the instants they were decided, not from when the callback ran: a late-scheduled
    // callback followed by an on-time one would show a gap the rule never made.
    let dir = tempfile::tempdir().expect("tmp");
    let c = cfg(dir.path(), Duration::from_secs(60), false);
    let max_delay = c.max_delay;
    let (tx, rx) = mpsc::channel();
    let stamps = Arc::new(std::sync::Mutex::new(Vec::<Instant>::new()));
    let sink = Arc::clone(&stamps);
    let on_decided = move |_, at| {
        sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(at);
    };
    let t = std::thread::spawn(move || {
        run_decided::<notify::NullWatcher>(&c, None, 0, &rx, &on_decided);
    });
    let rescan = || notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
    let storm_end = Instant::now() + max_delay * 3;
    let mut sent = 0;
    while Instant::now() < storm_end {
        tx.send(Msg::Event(Ok(rescan()))).expect("send");
        sent += 1;
        pause_blocking(Duration::from_millis(5));
    }
    // A notice after the first is still answered, once its `max_delay` is up.
    pause_blocking(max_delay * 2);
    tx.send(Msg::Stop).expect("stop");
    t.join().expect("join");
    let at = stamps.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    assert!(at.len() >= 2, "the first nudge is at once and the owed one follows: {at:?}");
    assert!(at.len() < sent, "{} nudges for {sent} notices", at.len());
    let tightest = at.windows(2).map(|w| w[1] - w[0]).min().expect("a gap");
    assert!(
        tightest >= max_delay,
        "nudges came {tightest:?} apart, less than the {max_delay:?} between them"
    );
}

#[test]
fn without_os_events_the_safety_timer_still_nudges() {
    let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    let (n, f) = counting();
    let _w = Nudger::start(cfg(dir.path(), Duration::from_millis(300), false), f).expect("w");
    assert!(wait_for(&n, 2, Duration::from_millis(1500)), "timer nudges");
}

#[test]
fn dropping_the_watcher_ends_its_thread() {
    let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    let (n, f) = counting();
    let w = Nudger::start(cfg(dir.path(), Duration::from_millis(50), true), f).expect("w");
    assert_eq!(Arc::strong_count(&n), 2, "the thread holds the callback");
    drop(w);
    assert_eq!(Arc::strong_count(&n), 1, "the callback is gone with the thread");
}

/// Class guard: a test that starts a real OS watcher and waits for a nudge must bound the wait
/// by the rescan interval. Waiting on the OS event itself flakes on a loaded machine, because
/// the stream may deliver late or drop events. Events fed by hand are for coalescing tests.
#[test]
fn every_real_watcher_test_waits_by_the_rescan_bound() {
    let source = include_str!("tests.rs");
    let mut checked = 0;
    for part in source.split("\n#[test]\n").skip(1) {
        let name = part.lines().next().unwrap_or_default();
        if name.contains("every_real_watcher") {
            continue;
        }
        let real = part.contains("Nudger::start(") && part.contains("true)");
        if !real {
            continue;
        }
        let waits = part.lines().filter(|l| l.contains("wait_for(")).count();
        let bounded = part
            .lines()
            .filter(|l| l.contains("wait_for("))
            .filter(|l| l.contains("rescan"))
            .count();
        if waits == 0 {
            continue;
        }
        checked += 1;
        assert_eq!(waits, bounded, "{name} waits on a real watcher without a rescan bound");
    }
    assert!(checked >= 2, "the guard found only {checked} real watcher tests");
}
