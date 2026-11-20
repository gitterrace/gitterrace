//! Unit tests for the blob diff.

use std::time::{Duration, Instant};

use crate::diff::{Window, stream};

fn collect<R: Send>(
    items: &[u64],
    workers: usize,
    window: usize,
    work: impl Fn(&u64) -> crate::Result<R> + Sync,
) -> crate::Result<Vec<R>> {
    let mut out = Vec::new();
    stream(
        items,
        workers,
        Window {
            items: window,
            bytes: usize::MAX,
            first: &[],
            budget: None,
        },
        || Ok(()),
        |(), i| work(i),
        |_| 0,
        |n, r| {
            assert_eq!(n, out.len(), "emitted out of order");
            out.push(r);
            Ok(())
        },
    )?;
    Ok(out)
}

/// One slow item must not hold back the items queued behind it: the others drain on free
/// threads while it runs. Counted, not timed: item 0 waits until the other 99 are done, which
/// only a shared queue allows (a fixed split runs items 1..25 after item 0 on its thread, a
/// barrier per batch waits for it), so either of those never finishes and the bound trips.
#[test]
fn one_slow_item_does_not_serialize_the_rest() -> crate::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let items: Vec<u64> = (0..100).collect();
    let done = AtomicUsize::new(0);
    let t0 = Instant::now();
    let out = collect(&items, 4, 100, |i| {
        if *i == 0 {
            // Generous: a loaded runner is slow, but never this slow at 99 sleeps of 20 ms.
            while done.load(Ordering::Acquire) < 99 && t0.elapsed() < Duration::from_secs(30) {
                gitterrace_testkit::wait::pause_blocking(Duration::from_millis(5));
            }
        } else {
            gitterrace_testkit::wait::pause_blocking(Duration::from_millis(20));
            done.fetch_add(1, Ordering::Release);
        }
        Ok(i * 2)
    })?;
    assert_eq!(out, items.iter().map(|i| i * 2).collect::<Vec<_>>());
    assert_eq!(
        done.load(Ordering::Acquire),
        99,
        "item 0 held back the queue for {:?}",
        t0.elapsed()
    );
    assert!(
        t0.elapsed() < Duration::from_secs(30),
        "the others waited on item 0"
    );
    Ok(())
}

/// Workers never run more than `window` items ahead of the oldest one not yet emitted, so
/// the results held for reordering behind a slow item stay bounded.
#[test]
fn workers_stay_inside_the_window() -> crate::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let items: Vec<u64> = (0..400).collect();
    let started = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    collect(&items, 8, 16, |i| {
        started.fetch_add(1, Ordering::SeqCst);
        if *i == 0 {
            gitterrace_testkit::wait::pause_blocking(Duration::from_millis(200));
            peak.store(started.load(Ordering::SeqCst), Ordering::SeqCst);
        }
        Ok(())
    })?;
    assert!(
        peak.load(Ordering::SeqCst) <= 16,
        "{}",
        peak.load(Ordering::SeqCst)
    );
    Ok(())
}

/// While the held results exceed the byte budget, only the item next to emit may start,
/// so a run of huge results behind a slow item cannot fill the whole (wide) count window.
/// Catches a budget that is never checked, or one checked only against the count.
#[test]
fn held_results_stay_inside_the_byte_budget() -> crate::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let items: Vec<u64> = (0..400).collect();
    let started = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    stream(
        &items,
        8,
        Window {
            items: 1000,
            bytes: 10 * 100,
            first: &[],
            budget: None,
        },
        || Ok(()),
        |(), i| {
            started.fetch_add(1, Ordering::SeqCst);
            if *i == 0 {
                gitterrace_testkit::wait::pause_blocking(Duration::from_millis(200));
                peak.store(started.load(Ordering::SeqCst), Ordering::SeqCst);
            }
            Ok(())
        },
        |()| 100,
        |_, ()| Ok(()),
    )?;
    // Budget 10 results, plus one per worker that passed the check before it filled.
    let peak = peak.load(Ordering::SeqCst);
    assert!(peak <= 10 + 1 + 8, "{peak} started behind the slow item");
    Ok(())
}

/// Runs [`stream`] over 0..n with `first` on a thread of its own, records the order the items
/// start in, and checks the results come out in input order. Waits on the run finishing, so a
/// scheduler that can stall fails on the deadline instead of hanging the suite.
fn start_order(n: u64, workers: usize, span: usize, bytes: usize, first: Vec<usize>) -> Vec<u64> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let started = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new(AtomicBool::new(false));
    let (s, d) = (Arc::clone(&started), Arc::clone(&done));
    let run = std::thread::spawn(move || {
        let items: Vec<u64> = (0..n).collect();
        let mut out = Vec::new();
        let r = stream(
            &items,
            workers,
            Window {
                items: span,
                bytes,
                first: &first,
                budget: None,
            },
            || Ok(()),
            |(), i| {
                s.lock()
                    .map_err(|e| crate::Error::Git(e.to_string()))?
                    .push(*i);
                Ok(*i)
            },
            |_| 100,
            |n, r| {
                out.push((n, r));
                Ok(())
            },
        );
        d.store(true, Ordering::SeqCst);
        (r.is_ok(), out)
    });
    gitterrace_testkit::wait::until_blocking("the stream to finish", || {
        done.load(Ordering::SeqCst)
    });
    let (ok, out) = run.join().unwrap_or_default();
    assert!(ok, "the stream failed");
    let want: Vec<(usize, u64)> = (0..n).map(|i| (i as usize, i)).collect();
    assert_eq!(out, want, "emitted out of order");
    started.lock().map(|s| s.clone()).unwrap_or_default()
}

/// The items named first start before every other item the window lets in, and the output
/// keeps the input order. Catches a `first` that is ignored, or one that reorders the output.
#[test]
fn items_named_first_start_ahead_of_the_rest() {
    let order = start_order(50, 1, 1000, usize::MAX, vec![7, 30, 49]);
    assert_eq!(order[..3], [7, 30, 49], "{order:?}");
    let rest: Vec<u64> = (0..50).filter(|i| ![7, 30, 49].contains(i)).collect();
    assert_eq!(order[3..], rest[..]);
}

/// With a tiny window and byte budget and the items named first far past it, the run still
/// finishes on any worker count: the item next to emit can always start, from whichever queue
/// holds it. Catches a scheduler that waits on the head of `first` while the window is full
/// of earlier items no worker may start: every worker blocked, the collector waiting on an
/// item nobody runs.
#[test]
fn a_tiny_window_with_late_first_items_still_finishes() {
    for workers in [1, 2, 8] {
        let order = start_order(100, workers, 2, 0, (90..100).collect());
        let mut sorted = order;
        sorted.sort_unstable();
        assert_eq!(sorted, (0..100).collect::<Vec<_>>(), "{workers} workers");
    }
    // A position past the end is ignored, never waited for.
    let order = start_order(10, 4, 2, 0, vec![3, 50]);
    assert_eq!(order.len(), 10);
}

/// Guard class: a worker limit lowered mid-run that the workers ignore, whether never read
/// or read only once at the start. Eight workers start and the budget drops to two once item
/// 50 is emitted. A worker that passed its check just before the drop still finishes the one
/// item it took, and the 8-item window keeps those below item 80, so from item 80 on at most
/// two run at once. The budget rises again at item 300 and more than two run. Output keeps
/// the input order throughout. Counted, never timed.
#[test]
fn a_budget_lowered_mid_run_shrinks_the_live_workers() -> crate::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let items: Vec<u64> = (0..400).collect();
    let budget = crate::Budget::new(8);
    let (running, peak_low, peak_high) = (
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    );
    let mut out = Vec::new();
    stream(
        &items,
        8,
        Window {
            items: 8,
            bytes: usize::MAX,
            first: &[],
            budget: Some(&budget),
        },
        || Ok(()),
        |(), i| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            match *i {
                80..300 => peak_low.fetch_max(now, Ordering::SeqCst),
                320.. => peak_high.fetch_max(now, Ordering::SeqCst),
                _ => 0,
            };
            // Long enough that the other workers are mid-item too.
            gitterrace_testkit::wait::pause_blocking(Duration::from_millis(2));
            running.fetch_sub(1, Ordering::SeqCst);
            Ok(*i)
        },
        |_| 0,
        |n, r| {
            match n {
                50 => budget.set(2),
                300 => budget.set(8),
                _ => {}
            }
            out.push(r);
            Ok(())
        },
    )?;
    assert_eq!(out, items);
    let low = peak_low.load(Ordering::SeqCst);
    assert!(low <= 2, "{low} workers ran under a budget of two");
    let high = peak_high.load(Ordering::SeqCst);
    assert!(
        high > 2,
        "the raised budget woke no parked worker: peak {high}"
    );
    Ok(())
}

/// One live worker of four under a tiny window still finishes: the three parked ones leave
/// when the run ends instead of holding the scope open. Catches an end that wakes only the
/// window's waiters, not the budget's.
#[test]
fn a_run_ends_with_every_parked_worker_released() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let done = Arc::new(AtomicBool::new(false));
    let d = Arc::clone(&done);
    let run = std::thread::spawn(move || {
        let items: Vec<u64> = (0..20).collect();
        let budget = crate::Budget::new(1);
        let mut out = Vec::new();
        let r = stream(
            &items,
            4,
            Window {
                items: 2,
                bytes: 0,
                first: &[],
                budget: Some(&budget),
            },
            || Ok(()),
            |(), i| Ok(*i),
            |_| 1,
            |_, r| {
                out.push(r);
                Ok(())
            },
        );
        d.store(true, Ordering::SeqCst);
        r.is_ok() && out == items
    });
    // A deadline instead of a hung suite when a parked worker is never released.
    gitterrace_testkit::wait::until_blocking("the run to end", || done.load(Ordering::SeqCst));
    assert!(run.join().unwrap_or(false), "the run failed or reordered");
}

#[test]
fn results_keep_input_order_and_errors_surface() {
    let items: Vec<u64> = (0..1000).collect();
    assert!(collect(&items, 7, 32, |i| Ok(*i)).is_ok_and(|o| o == items));
    let err = collect(&items, 3, 32, |i| {
        if *i == 500 {
            Err(crate::Error::Git("boom".into()))
        } else {
            Ok(())
        }
    });
    assert!(err.is_err());
    let failed_emit = stream(
        &items,
        4,
        Window {
            items: 8,
            bytes: usize::MAX,
            first: &[],
            budget: None,
        },
        || Ok(()),
        |(), _| Ok(()),
        |()| 0,
        |n, ()| {
            if n == 10 {
                Err(crate::Error::Git("disk".into()))
            } else {
                Ok(())
            }
        },
    );
    assert!(failed_emit.is_err());
    let failed_setup = stream(
        &items,
        4,
        Window {
            items: 8,
            bytes: usize::MAX,
            first: &[],
            budget: None,
        },
        || Err::<(), _>(crate::Error::Git("x".into())),
        |(), _| Ok(()),
        |()| 0,
        |_, ()| Ok(()),
    );
    assert!(failed_setup.is_err());
    assert!(collect(&[], 4, 8, |_| Ok(())).is_ok_and(|o: Vec<()>| o.is_empty()));
}

/// Every commit diffed with the blob-pair memo (shared, and so small it starts over many
/// times) has the same file stats as without it. Catches a memo key that misses an input
/// the counts depend on: a pure move and an edit of the same pair, an addition and a
/// deletion of one blob, or a hit returning another pair's counts.
#[test]
fn the_count_memo_never_changes_a_result() -> crate::Result<()> {
    let dir = tempfile::tempdir().map_err(crate::error::git)?;
    let mut f = gitterrace_testkit::Fixture::init(dir.path());
    let body = (0..30).fold(String::new(), |b, l| b + &format!("line {l}\n"));
    for round in 0..4 {
        f = f
            .write("a.txt", &body)
            .write("b.txt", &format!("{body}round {round}\n"))
            .commit("same blobs again");
        f.git(&["mv", "a.txt", "moved.txt"]);
        f = f.commit("pure move");
        f.git(&["rm", "-q", "moved.txt"]);
        f = f.write("c.txt", " spaced  \n\n").commit("delete and add");
        // One new blob reached from two different old blobs, in two files.
        f = f
            .write("x.txt", "one\ntwo\n")
            .write("y.txt", "one\nthree\nfour\n")
            .commit("two starts");
        f = f
            .write("x.txt", "one\nend\n")
            .write("y.txt", "one\nend\n")
            .commit("one end");
    }
    let repo = crate::Repo::discover(dir.path())?;
    let gix = repo.repo();
    let memo = super::CountMemo::new(3);
    let mut cache = super::resource_cache(&gix)?;
    for id in repo.rev_list("HEAD")? {
        let commit = gix
            .find_commit(repo.full_id(&gix, &id)?)
            .map_err(crate::error::git)?;
        let plain = super::changes(&gix, &commit, false, crate::MAX_TEXT_BYTES)?;
        for _ in 0..2 {
            let memoized = super::changes_with(
                &gix,
                &commit,
                false,
                crate::MAX_TEXT_BYTES,
                &mut cache,
                Some(&memo),
                super::Renames::Full,
            )?;
            assert_eq!(format!("{memoized:?}"), format!("{plain:?}"), "commit {id}");
        }
    }
    Ok(())
}

/// The cache built from an owned index reads the same attributes as the gix shortcut, so
/// every diff of a history with `binary`, `diff=`, `-diff` and `text` rules, renames and
/// edits across them is the same. Red when the attributes are dropped (an empty index).
#[test]
fn the_owned_index_cache_diffs_as_the_shared_one() -> crate::Result<()> {
    let dir = tempfile::tempdir().map_err(crate::error::git)?;
    let mut f = gitterrace_testkit::Fixture::init(dir.path());
    let body = (0..40).fold(String::new(), |b, l| b + &format!("line {l}\n"));
    f = f
        .write(
            ".gitattributes",
            "*.dat binary\n*.md diff=markdown\n*.gen -diff\n*.txt text eol=crlf\n",
        )
        .write("a.dat", &body)
        .write("b.md", &body)
        .write("c.gen", &body)
        .write("d.txt", &body)
        .commit("attributes");
    // Attributes reach the result through rename similarity: a `binary` or `-diff` file
    // moved with an edit is no rename to compare by content, a text one is.
    f.git(&["mv", "a.dat", "moved.dat"]);
    f.git(&["mv", "c.gen", "moved.gen"]);
    f.git(&["mv", "b.md", "moved.md"]);
    f = f
        .write("moved.dat", &format!("{body}more\n"))
        .write("moved.gen", &format!("{body}more\n"))
        .write("moved.md", &format!("{body}more\n"))
        .write("d.txt", &format!("x\n{body}"))
        .commit("renames and edits");
    f.git(&["mv", "moved.gen", "e.txt"]);
    let _done = f
        .write("e.txt", &format!("{body}again\n"))
        .commit("across rules");
    let repo = crate::Repo::discover(dir.path())?;
    let gix = repo.repo();
    let mut owned = super::resource_cache(&gix)?;
    let mut shared = gix
        .diff_resource_cache_for_tree_diff()
        .map_err(crate::error::git)?;
    for id in repo.rev_list("HEAD")? {
        let commit = gix
            .find_commit(repo.full_id(&gix, &id)?)
            .map_err(crate::error::git)?;
        for hunks in [false, true] {
            let a = super::changes_with(
                &gix,
                &commit,
                hunks,
                1 << 20,
                &mut owned,
                None,
                super::Renames::Full,
            )?;
            let b = super::changes_with(
                &gix,
                &commit,
                hunks,
                1 << 20,
                &mut shared,
                None,
                super::Renames::Full,
            )?;
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "commit {id}");
        }
    }
    Ok(())
}

/// A diff computed on longer texts than the ones it is applied to: its ranges run past them.
fn stale_diff() -> gix::diff::blob::Diff {
    use gix::diff::blob::{Algorithm, Diff, InternedInput};
    let (old, new) = (
        b"a\nb\nc\nd\ne\nf\n".as_slice(),
        b"a\nb\nc\nd\ne\nF\n".as_slice(),
    );
    Diff::compute(Algorithm::Myers, &InternedInput::new(old, new))
}

#[test]
fn code_counts_ignores_out_of_range_hunk() {
    let input = gix::diff::blob::InternedInput::new(b"a\n".as_slice(), b"b\n".as_slice());
    let counts = crate::diff::code_counts(&input, &stale_diff());
    assert_eq!(counts, crate::CodeCounts::default());
}

#[test]
fn build_hunks_ignores_out_of_range_hunk() {
    assert!(crate::diff::hunks::build_hunks(b"a\n", b"b\n", &stale_diff()).is_empty());
}

#[test]
fn stream_workers_run_the_installed_hook_on_and_off() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ON: AtomicUsize = AtomicUsize::new(0);
    static OFF: AtomicUsize = AtomicUsize::new(0);
    crate::set_worker_hook(|on| {
        if on { &ON } else { &OFF }.fetch_add(1, Ordering::Relaxed);
    });
    let out = collect(&[1, 2, 3, 4], 3, 8, |i| Ok(*i));
    assert!(out.is_ok());
    // Other tests stream on their own workers meanwhile, so these are floors.
    assert!(ON.load(Ordering::Relaxed) >= 3, "one on per worker");
    assert!(OFF.load(Ordering::Relaxed) >= 3, "one off per worker");
}
