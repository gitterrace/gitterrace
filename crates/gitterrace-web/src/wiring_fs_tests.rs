//! Working tree scans follow OS file events: fast on a change, silent while idle, one scan per
//! burst, and the safety rescan still finds a change no event reported.

use std::sync::{Arc, atomic::Ordering};
use std::time::{Duration, Instant};

use crate::app::App;
use crate::live::WorktreeWatch;
use crate::wiring_tests::{app, repo, with};

/// These tests count scans and time file events, so they run one at a time: a parallel test
/// writing thousands of files would slow the OS event stream they measure.
pub(crate) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn scans(a: &App) -> u64 {
    a.wt_scans.load(Ordering::Relaxed)
}

/// A served repository with a tab on the worktree page, settled past its first scan.
async fn watched(
    dir: &std::path::Path,
    rescan_secs: u64,
    os_events: bool,
) -> (
    Arc<App>,
    std::path::PathBuf,
    tokio::sync::broadcast::Receiver<Arc<crate::live::Change>>,
    WorktreeWatch,
    tokio::task::AbortHandle,
) {
    let p = repo(dir, "r");
    let a = app(&p, with(|c| c.watch.rescan_secs = rescan_secs));
    a.os_events.store(os_events, Ordering::Release);
    let task = tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&a))).abort_handle();
    let mut rx = a.changes.subscribe();
    let wt = WorktreeWatch::new(&a);
    // FSEvents may replay the fixture's own writes late and in one batch, on a loaded machine
    // after any quiet second. Events arrive in order, so once a new untracked marker shows as
    // a worktree change (the replayed git writes never do), that backlog is delivered.
    if os_events {
        let end = Instant::now() + Duration::from_secs(30);
        let mut n = 0;
        while Instant::now() < end {
            n += 1;
            std::fs::write(p.join(format!("settle-{n}.txt")), "x\n").expect("marker");
            if worktree_change(&mut rx, Duration::from_secs(3)).await {
                break;
            }
        }
    }
    let end = Instant::now() + Duration::from_secs(10);
    let mut last = u64::MAX;
    while Instant::now() < end {
        let now = scans(&a);
        if now == last && now > 0 {
            break;
        }
        last = now;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    (a, p, rx, wt, task)
}

async fn worktree_change(
    rx: &mut tokio::sync::broadcast::Receiver<Arc<crate::live::Change>>,
    within: Duration,
) -> bool {
    let end = tokio::time::Instant::now() + within;
    while let Ok(Ok(c)) = tokio::time::timeout_at(end, rx.recv()).await {
        if c.kinds.contains(&"worktree") {
            return true;
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_shows_within_the_default_interval() {
    let _one = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let (_a, p, mut rx, _wt, _t) = watched(dir.path(), 30, true).await;
    // The best of three edits, so a loaded test machine stalling one scan does not decide it;
    // every edit must still arrive.
    let mut best = Duration::MAX;
    for i in 0..3 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        while rx.try_recv().is_ok() {}
        // Write again every few seconds: a loaded machine may drop one event, which the safety
        // rescan covers in production but this test does not wait for.
        let mut seen = false;
        for n in 0..8 {
            std::fs::write(p.join("small.txt"), format!("edited {i} {n}\n")).expect("write");
            let t0 = Instant::now();
            if worktree_change(&mut rx, Duration::from_secs(3)).await {
                best = best.min(t0.elapsed());
                seen = true;
                break;
            }
        }
        assert!(seen, "edit {i} reaches the tab");
    }
    assert!(best < Duration::from_millis(1500), "an edit took {best:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_tree_is_not_scanned_and_a_touched_one_is() {
    let _one = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let (a, p, _rx, _wt, _t) = watched(dir.path(), 30, true).await;
    let before = scans(&a);
    tokio::time::sleep(Duration::from_secs(10)).await;
    // Polling would scan once per interval (1 s), ten times here, and a scan that wakes the
    // watcher would keep looping. A heavily loaded machine still delivers a few late
    // duplicates of the settle markers' events, so up to three stray scans pass and the
    // regressions, at ten or more, do not.
    let idle = scans(&a) - before;
    assert!(idle <= 3, "{idle} scans while nothing changes");
    // The positive control: every touch is followed by a scan of its own. Each waits for the
    // previous scan to finish, so a loaded machine folding touches together cannot decide it.
    // The safety rescan alone (30 s) would need far longer than the total allowed.
    let t0 = Instant::now();
    for i in 0..10 {
        let before = scans(&a);
        std::fs::write(p.join("small.txt"), format!("touch {i}\n")).expect("write");
        assert!(
            scanned_after(&a, before, Duration::from_secs(25)).await,
            "touch {i} is scanned"
        );
    }
    let took = t0.elapsed();
    assert!(took < Duration::from_secs(60), "10 touches took {took:?}");
}

/// Waits until a scan finished after the count read `before`.
async fn scanned_after(a: &App, before: u64, within: Duration) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if scans(a) > before {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn without_file_events_the_safety_rescan_finds_the_change() {
    let _one = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let (_a, p, mut rx, _wt, _t) = watched(dir.path(), 5, false).await;
    while rx.try_recv().is_ok() {}
    std::fs::write(p.join("small.txt"), "missed by events\n").expect("write");
    assert!(
        worktree_change(&mut rx, Duration::from_secs(20)).await,
        "the rescan finds what no event reported"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_five_thousand_files_is_a_few_scans() {
    let _one = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let (a, p, _rx, _wt, _t) = watched(dir.path(), 30, true).await;
    let before = scans(&a);
    let out = p.join("gen");
    std::fs::create_dir(&out).expect("dir");
    let t0 = Instant::now();
    for i in 0..5000 {
        std::fs::write(out.join(format!("f{i}.txt")), "x").expect("write");
    }
    assert!(
        scanned_after(&a, before, Duration::from_secs(20)).await,
        "the burst is scanned"
    );
    // Settled once two seconds pass without a scan; `busy` is how long scans kept coming.
    let (mut last, mut moved) = (scans(&a), Instant::now());
    while moved.elapsed() < Duration::from_secs(2) && t0.elapsed() < Duration::from_secs(120) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if scans(&a) != last {
            (last, moved) = (scans(&a), Instant::now());
        }
    }
    let busy = moved - t0;
    let got = last - before;
    // At most one scan per `MAX_DELAY` (500 ms) while events keep arriving, plus the tail and
    // an overflow notice. A loaded machine that delivers events late earns a longer window,
    // but a scan per file (thousands) never fits it.
    let most = busy.as_millis() / 500 + 6;
    assert!(
        u128::from(got) <= most,
        "{got} scans for 5000 files over {busy:?}, most {most}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_repository_leaves_nothing_running() {
    let _one = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let (a, p, rx, wt, task) = watched(dir.path(), 30, true).await;
    task.abort();
    drop((rx, wt));
    let end = Instant::now() + Duration::from_secs(3);
    while Arc::strong_count(&a) > 1 && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        Arc::strong_count(&a),
        1,
        "the refresher and its watcher let go"
    );
    let before = scans(&a);
    std::fs::write(p.join("small.txt"), "after removal\n").expect("write");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(scans(&a), before, "no scan after removal");
}
