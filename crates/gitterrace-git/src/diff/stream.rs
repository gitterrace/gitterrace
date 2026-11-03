//! Ordered parallel map over commits with a bounded reorder window.

use crate::Result;

/// How far [`stream`]'s workers may run ahead of the oldest result not yet emitted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Window {
    /// Items started past the next one to emit.
    pub(crate) items: usize,
    /// Estimated bytes of finished results held for reordering.
    pub(crate) bytes: usize,
}

/// What the workers of [`stream`] wait on.
#[derive(Default)]
struct Ahead {
    next: usize,
    held: usize,
}

/// Runs `work` over `items` on `workers` threads that each take the next item when free, and
/// hands each result to `emit` in the order of `items` as soon as it and every earlier one are
/// done. `setup` builds one worker's state (a repository handle, a blob cache) once. Commit cost
/// is very uneven: one mass rename can take seconds while its neighbours take microseconds, so
/// a fixed split per thread, or a barrier per batch, would leave the other cores idle behind
/// the slowest share. Here a slow item holds back only the emitting, never the free workers. A worker may run at most `window` items ahead of the next one to
/// emit, so results waiting in the reorder buffer stay bounded however slow one item is.
/// The window is also bounded in bytes: `weight` estimates a result's size, and while the
/// results held exceed `budget` only the item next to emit may start. So a run of huge results
/// (a merge window's worth of large merges) cannot hold `window` times their size.
/// The first error (from a worker or from `emit`) stops the run and is returned.
pub(crate) fn stream<T: Sync, S, R: Send>(
    items: &[T],
    workers: usize,
    window: Window,
    setup: impl Fn() -> Result<S> + Sync,
    work: impl Fn(&mut S, &T) -> Result<R> + Sync,
    weight: impl Fn(&R) -> usize + Sync,
    mut emit: impl FnMut(usize, R) -> Result<()>,
) -> Result<()> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex, mpsc};

    let Window {
        items: span,
        bytes: budget,
    } = window;
    let span = span.max(1);
    let workers = workers.clamp(1, items.len().max(1));
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    // The next item to emit and the bytes of finished results not yet emitted; workers wait
    // on them to stay inside the window.
    let emitted = (Mutex::new(Ahead::default()), Condvar::new());
    // A worker counts its result's bytes before sending it, so results still in the channel
    // count too; the channel itself never blocks a worker.
    let (tx, rx) = mpsc::channel::<(usize, Result<R>, usize)>();
    let release = |next: usize, freed: usize| {
        if let Ok(mut e) = emitted.0.lock() {
            e.next = next;
            e.held = e.held.saturating_sub(freed);
        }
        emitted.1.notify_all();
    };
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (next, stop, emitted, setup, work, weight) =
                (&next, &stop, &emitted, &setup, &work, &weight);
            scope.spawn(move || {
                let mut state = match setup() {
                    Ok(s) => s,
                    Err(e) => {
                        drop(tx.send((usize::MAX, Err(e), 0)));
                        return;
                    }
                };
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { return };
                    let Ok(mut e) = emitted.0.lock() else { return };
                    // The item next to emit always runs, or a full budget would wait forever.
                    while i > e.next
                        && (i >= e.next.saturating_add(span) || e.held > budget)
                        && !stop.load(Ordering::Relaxed)
                    {
                        let Ok(g) = emitted.1.wait(e) else { return };
                        e = g;
                    }
                    drop(e);
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let r = work(&mut state, item);
                    let w = r.as_ref().map_or(0, weight);
                    if let Ok(mut e) = emitted.0.lock() {
                        e.held = e.held.saturating_add(w);
                    }
                    if tx.send((i, r, w)).is_err() {
                        return;
                    }
                }
            });
        }
        drop(tx);
        let mut held = std::collections::BTreeMap::new();
        let mut want = 0;
        let mut out = Ok(());
        while want < items.len() {
            let Ok((i, r, w)) = rx.recv() else {
                out = Err(crate::Error::Git("diff worker panicked".to_string()));
                break;
            };
            held.insert(i, (r, w));
            let before = want;
            let mut freed = 0_usize;
            while let Some((r, w)) = held.remove(&want) {
                freed = freed.saturating_add(w);
                if let Err(e) = r.and_then(|r| emit(want, r)) {
                    out = Err(e);
                    break;
                }
                want += 1;
            }
            if out.is_err() {
                break;
            }
            // A worker that could not start reports under usize::MAX.
            if let Some((Err(e), _)) = held.remove(&usize::MAX) {
                out = Err(e);
                break;
            }
            if want != before {
                release(want, freed);
            }
        }
        if out.is_err() {
            stop.store(true, Ordering::Relaxed);
            release(usize::MAX, 0);
        }
        drop(rx);
        out
    })
}
