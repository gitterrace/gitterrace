//! Ordered parallel map over commits with a bounded reorder window.

use crate::Result;

/// How far [`stream`]'s workers may run ahead of the oldest result not yet emitted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Window<'a> {
    /// Items started past the next one to emit.
    pub(crate) items: usize,
    /// Estimated bytes of finished results held for reordering.
    pub(crate) bytes: usize,
    /// Positions to start ahead of the others, ascending, see [`stream`].
    pub(crate) first: &'a [usize],
}

/// What the workers of [`stream`] wait on, and the two queues they take items from.
struct Ahead<'a> {
    next: usize,
    held: usize,
    /// Positions taken first, ascending, and how many of them are taken.
    first: &'a [usize],
    first_taken: usize,
    /// Marks `first`'s positions, which the ascending queue skips.
    is_first: Vec<bool>,
    /// The ascending queue: the smallest position not yet looked at.
    rest: usize,
}

impl<'a> Ahead<'a> {
    fn new(len: usize, first: &'a [usize]) -> Self {
        let mut is_first = vec![false; len];
        for &i in first {
            if let Some(m) = is_first.get_mut(i) {
                *m = true;
            }
        }
        let first_len = first.partition_point(|&i| i < len);
        Self {
            next: 0,
            held: 0,
            first: &first[..first_len],
            first_taken: 0,
            is_first,
            rest: 0,
        }
    }

    /// The head of the ascending queue, skipping `first`'s positions.
    fn rest_head(&mut self) -> usize {
        while self.is_first.get(self.rest).copied().unwrap_or(false) {
            self.rest += 1;
        }
        self.rest
    }

    /// The next item a worker may start: the head of `first` when it fits, else the head of
    /// the ascending queue when it fits. `None` while neither fits, `Some(len)` when both are
    /// empty. Both queues are ascending and every position below `next` is taken, so an untaken
    /// `next` is the head of one of them, and it always fits: some worker can always start it.
    /// That keeps the run moving however small the window and however late `first` reaches.
    fn take(&mut self, len: usize, span: usize, budget: usize) -> Option<usize> {
        let fits = |e: &Self, i: usize| {
            i == e.next || (i < e.next.saturating_add(span) && e.held <= budget)
        };
        if let Some(&i) = self.first.get(self.first_taken)
            && fits(self, i)
        {
            self.first_taken += 1;
            return Some(i);
        }
        let i = self.rest_head();
        if i >= len {
            return (self.first_taken >= self.first.len()).then_some(len);
        }
        if fits(self, i) {
            self.rest += 1;
            return Some(i);
        }
        None
    }
}

/// Runs `work` over `items` on `workers` threads that each take the next item when free, and
/// hands each result to `emit` in the order of `items` as soon as it and every earlier one are
/// done. `setup` builds one worker's state (a repository handle, a blob cache) once. Commit cost
/// is very uneven: one mass rename can take seconds while its neighbours take microseconds, so
/// a fixed split per thread, or a barrier per batch, would leave the other cores idle behind
/// the slowest share. Here a slow item holds back only the emitting, never the free workers. A
/// worker may run at most `window` items ahead of the next one to emit, so results waiting in the
/// reorder buffer stay bounded however slow one item is. The window is also bounded in bytes:
/// `weight` estimates a result's size, and while the results held exceed `budget` only the item
/// next to emit may start. So a run of huge results (a merge window's worth of large merges) cannot
/// hold `window` times their size. The window's `first` (ascending positions, the expensive
/// items) start ahead of the others as far as the window allows, so a slow item begins early
/// instead of holding back the emitting when its turn comes. Results still reach `emit` in the
/// order of `items`. The first error (from a worker or from `emit`) stops the run and is returned.
pub(crate) fn stream<T: Sync, S, R: Send>(
    items: &[T],
    workers: usize,
    window: Window<'_>,
    setup: impl Fn() -> Result<S> + Sync,
    work: impl Fn(&mut S, &T) -> Result<R> + Sync,
    weight: impl Fn(&R) -> usize + Sync,
    mut emit: impl FnMut(usize, R) -> Result<()>,
) -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Condvar, Mutex, mpsc};

    let Window {
        items: span,
        bytes: budget,
        first,
    } = window;
    let span = span.max(1);
    let workers = workers.clamp(1, items.len().max(1));
    let len = items.len();
    let stop = AtomicBool::new(false);
    // The next item to emit and the bytes of finished results not yet emitted; workers wait
    // on them to stay inside the window.
    let emitted = (Mutex::new(Ahead::new(len, first)), Condvar::new());
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
            let (stop, emitted, setup, work, weight) = (&stop, &emitted, &setup, &work, &weight);
            scope.spawn(move || {
                let mut hook = crate::worker_hook::WorkerGuard::start();
                let mut state = match hook.busy(setup) {
                    Ok(s) => s,
                    Err(e) => {
                        drop(tx.send((usize::MAX, Err(e), 0)));
                        return;
                    }
                };
                loop {
                    let Ok(mut e) = emitted.0.lock() else { return };
                    let i = loop {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Some(i) = e.take(len, span, budget) {
                            break i;
                        }
                        let Ok(g) = emitted.1.wait(e) else { return };
                        e = g;
                    };
                    drop(e);
                    let Some(item) = items.get(i) else { return };
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let r = hook.busy(|| work(&mut state, item));
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
