//! Working tree scans follow OS file events: fast on a change, silent while idle, one scan per
//! burst, and the safety rescan still finds a change no event reported.
//!
//! The promise is documented: a change shows within `watch.rescan_secs` at the latest, because
//! the OS may drop an event on a loaded Mac. Tests set the shortest allowed rescan and assert
//! that bound plus a margin, so they never depend on the OS delivering an event on time.

use std::sync::{Arc, atomic::Ordering};
use std::time::{Duration, Instant};

use crate::app::App;
use crate::live::WorktreeWatch;
use crate::wiring_tests::{app, repo, with};
use gitterrace_testkit::git;

/// These tests count scans and time file events, so they run one at a time: a parallel test
/// writing thousands of files would slow the OS event stream they measure.
pub(crate) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The shortest `watch.rescan_secs` the config allows.
const RESCAN: u64 = 5;
/// Slack on top of the rescan for a busy machine: one scan and its delivery.
const MARGIN: Duration = Duration::from_secs(5);
/// How long a change may take to show: the documented bound.
const BOUND: Duration = Duration::from_secs(RESCAN + 5);

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
    // The fixture's own writes may still be replayed by the OS, late and in one batch. A new
    // untracked marker shows as a worktree change (the replayed git writes never do), by event
    // or at the latest by the safety rescan, so that backlog is out of the way afterwards.
    if os_events {
        let end = Instant::now() + Duration::from_secs(30);
        let mut n = 0;
        while Instant::now() < end {
            n += 1;
            std::fs::write(p.join(format!("settle-{n}.txt")), "x\n").expect("marker");
            if worktree_change(&mut rx, BOUND).await {
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
    let (_a, p, mut rx, _wt, _t) = watched(dir.path(), RESCAN, true).await;
    // One write per edit, no retry: the OS usually reports it within a second, and when it does
    // not, the safety rescan shows it within `rescan_secs`.
    for i in 0..3 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        while rx.try_recv().is_ok() {}
        std::fs::write(p.join("small.txt"), format!("edited {i}\n")).expect("write");
        let t0 = Instant::now();
        assert!(
            worktree_change(&mut rx, BOUND).await,
            "edit {i} not shown within {BOUND:?} (waited {:?})",
            t0.elapsed()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_tree_is_not_scanned_and_a_touched_one_is() {
    let _one = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let (a, p, mut rx, _wt, _t) = watched(dir.path(), RESCAN, true).await;
    let before = scans(&a);
    let window = Duration::from_secs(12);
    tokio::time::sleep(window).await;
    // Polling would scan once per interval (1 s), twelve times here, and a scan that wakes the
    // watcher would keep looping. The safety rescan adds one scan per `rescan_secs`, and a few
    // late duplicates of the settle markers' events may add more, so the idle count stays well
    // under what polling gives.
    let idle = scans(&a) - before;
    let most = window.as_secs() / RESCAN + 1 + 3;
    assert!(
        idle <= most,
        "{idle} scans in {window:?} while nothing changes"
    );
    // The positive control: every touch shows as a worktree change within the documented bound.
    for i in 0..3 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        while rx.try_recv().is_ok() {}
        std::fs::write(p.join("small.txt"), format!("touch {i}\n")).expect("write");
        assert!(
            worktree_change(&mut rx, BOUND).await,
            "touch {i} not shown within {BOUND:?}"
        );
    }
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
    let (a, p, _rx, _wt, _t) = watched(dir.path(), RESCAN, true).await;
    let before = scans(&a);
    let out = p.join("gen");
    std::fs::create_dir(&out).expect("dir");
    let t0 = Instant::now();
    for i in 0..5000 {
        std::fs::write(out.join(format!("f{i}.txt")), "x").expect("write");
    }
    assert!(
        scanned_after(&a, before, BOUND + MARGIN).await,
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

/// A served repository's refresher with no tab open, its first fingerprint taken.
async fn refresher(p: &std::path::Path, rescan_secs: u64, os_events: bool) -> Arc<crate::app::App> {
    let a = app(p, with(|c| c.watch.rescan_secs = rescan_secs));
    a.os_events
        .store(os_events, std::sync::atomic::Ordering::Release);
    tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&a)));
    let end = Instant::now() + Duration::from_secs(20);
    while a.version() == 0 && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    a
}

/// Guard class: HEAD and refs follow OS file events, not a timer. The safety rescan is an hour
/// here, so only an event can announce the new branch; red when the git directories go
/// unwatched. Retried, since a loaded machine may drop one event.
#[tokio::test]
async fn a_ref_update_shows_from_file_events_without_a_timer() {
    let _one = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let a = refresher(&p, 3600, true).await;
    let start = a.version();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut n = 0;
    while a.version() == start {
        assert!(Instant::now() < deadline, "no event in {n} tries");
        n += 1;
        git(&p, &["branch", &format!("b{n}")]);
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Guard class: a change no event reports still shows within `watch.rescan_secs`. Events are
/// off, so only the safety rescan can find the commit; red without it, and red if something
/// polls faster than the rescan while no tab is open.
#[tokio::test]
async fn a_dropped_event_still_shows_within_the_rescan() {
    let _one = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let rescan = 5;
    let a = refresher(&p, rescan, false).await;
    // Align with a rescan: the version moves only when one finds the commit.
    let start = a.version();
    git(
        &p,
        &["commit", "-q", "--allow-empty", "-m", "chore: unseen"],
    );
    let t0 = Instant::now();
    let bound = Duration::from_secs(rescan + 5);
    while a.version() == start && t0.elapsed() < bound {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_ne!(a.version(), start, "not shown within {bound:?}");
    // The next commit waits for the next rescan: nothing polls in between.
    let seen = a.version();
    git(
        &p,
        &["commit", "-q", "--allow-empty", "-m", "chore: unseen 2"],
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        a.version(),
        seen,
        "a change showed between rescans with events off"
    );
}

/// Guard class: a commit and a working tree edit reach an open tab's event stream within
/// `watch.rescan_secs`, with OS file events on and with them off (the rescan alone). Red when
/// the refresher sleeps through a git or worktree change, whatever wake path broke.
#[tokio::test]
async fn an_open_tab_hears_a_commit_and_an_edit_within_the_rescan() {
    let _one = SERIAL.lock().await;
    for os in [true, false] {
        let dir = tempfile::tempdir().expect("tmp");
        let p = repo(dir.path(), "r");
        let a = app(&p, with(|c| c.watch.rescan_secs = 5));
        a.os_events.store(os, std::sync::atomic::Ordering::Release);
        let task = tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&a)));
        // The tab opens on an idle refresher, as a browser does: it must wake for the tab.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        // What `/events?wt=1` holds: a subscription and a worktree watch.
        let mut rx = a.changes.subscribe();
        let _wt = crate::live::WorktreeWatch::new(&a);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let bound = Duration::from_secs(5 + 5);
        let mut hears = async |kind: &str| {
            let end = tokio::time::Instant::now() + bound;
            while let Ok(Ok(c)) = tokio::time::timeout_at(end, rx.recv()).await {
                if c.kinds.contains(&kind) {
                    return true;
                }
            }
            false
        };
        // The edit comes first: a scan that starts late takes it as its baseline and the tab
        // never hears it.
        std::fs::write(p.join("small.txt"), "edited\n").expect("write");
        assert!(
            hears("worktree").await,
            "events {os}: no edit within {bound:?}"
        );
        git(&p, &["commit", "-q", "--allow-empty", "-m", "chore: live"]);
        assert!(
            hears("refs").await,
            "events {os}: no commit within {bound:?}"
        );
        task.abort();
    }
}
