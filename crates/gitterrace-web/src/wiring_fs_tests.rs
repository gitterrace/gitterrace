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
    let rx = a.changes.subscribe();
    let wt = WorktreeWatch::new(&a);
    // Settled once no scan ran for a second: FSEvents may still report the fixture's own
    // writes from just before the watch started.
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
    assert_eq!(scans(&a), before, "no scan while nothing changes");
    // The positive control: the same window with a file touched every second scans.
    let before = scans(&a);
    for i in 0..10 {
        std::fs::write(p.join("small.txt"), format!("touch {i}\n")).expect("write");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let got = scans(&a) - before;
    assert!(got >= 5, "{got} scans for 10 touches");
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
    for i in 0..5000 {
        std::fs::write(out.join(format!("f{i}.txt")), "x").expect("write");
    }
    let end = Instant::now() + Duration::from_secs(20);
    while scans(&a) == before && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut last = u64::MAX;
    while scans(&a) != last {
        last = scans(&a);
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let got = last - before;
    // One scan per `MAX_DELAY` while the writes go on at most, nowhere near one per file.
    assert!((1..=20).contains(&got), "{got} scans for 5000 files");
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
