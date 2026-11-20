//! A hook the binary installs to run around each parallel index worker, and the time those
//! workers spend working.
//!
//! The binary runs jemalloc without thread caches, so an idle server's freed objects do not
//! pin pages. The index build allocates on every core at once and runs twice as slow without
//! them (git.git: 21.6 s against 10.8 s). Only the binary links jemalloc, so it hands this
//! crate a switch: on when a worker starts, off (which flushes the cache) when it ends.
//!
//! The busy share says whether more workers would help: a stream whose workers wait on the
//! reorder window most of the time is held back by one slow commit, not by the core count.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static HOOK: OnceLock<fn(bool)> = OnceLock::new();
static ALIVE: AtomicU64 = AtomicU64::new(0);
/// Nanoseconds of that spent in the work itself, not waiting on the window.
static BUSY: AtomicU64 = AtomicU64::new(0);

/// Time the parallel workers of this process spent, summed over all of them since it started.
///
/// Two readings bracket a step: [`WorkerTime::busy_share`] of their difference is the part of
/// the workers' lives in that step spent working.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkerTime {
    /// Lifetime of the workers that finished.
    pub alive: Duration,
    /// Time those workers spent in their work.
    pub busy: Duration,
}

impl WorkerTime {
    /// The totals so far. A worker counts once it ends.
    pub fn now() -> Self {
        Self {
            alive: Duration::from_nanos(ALIVE.load(Ordering::Relaxed)),
            busy: Duration::from_nanos(BUSY.load(Ordering::Relaxed)),
        }
    }

    /// The time spent between `earlier` and this reading.
    #[must_use]
    pub const fn since(self, earlier: Self) -> Self {
        Self {
            alive: self.alive.saturating_sub(earlier.alive),
            busy: self.busy.saturating_sub(earlier.busy),
        }
    }

    /// Busy over alive, `None` when no worker finished in between.
    pub fn busy_share(self) -> Option<f64> {
        (!self.alive.is_zero())
            .then(|| (self.busy.as_secs_f64() / self.alive.as_secs_f64()).min(1.0))
    }
}

/// Installs `hook`, called with `true` as an index worker thread starts and `false` before it
/// ends. The first install wins.
pub fn set_worker_hook(hook: fn(bool)) {
    // A second install keeps the first: one allocator per process.
    HOOK.get_or_init(|| hook);
}

/// Calls the hook with `on`, if one is installed.
pub(crate) fn worker(on: bool) {
    if let Some(h) = HOOK.get() {
        h(on);
    }
}

/// Runs the hook on now and off when dropped, so a worker that returns early still flushes,
/// and adds the worker's life and busy time to [`WorkerTime`] as it ends.
pub(crate) struct WorkerGuard {
    started: Instant,
    busy: Duration,
    /// Parked outside the worker budget: not part of the busy share, which measures the
    /// workers that may run.
    parked: Duration,
}

impl WorkerGuard {
    pub(crate) fn start() -> Self {
        worker(true);
        Self { started: Instant::now(), busy: Duration::ZERO, parked: Duration::ZERO }
    }

    pub(crate) fn busy<R>(&mut self, work: impl FnOnce() -> R) -> R {
        let t = Instant::now();
        let r = work();
        self.busy += t.elapsed();
        r
    }

    /// Waits outside the budget, counted as neither alive nor busy.
    pub(crate) fn idle<R>(&mut self, wait: impl FnOnce() -> R) -> R {
        let t = Instant::now();
        let r = wait();
        self.parked += t.elapsed();
        r
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let nanos = |d: Duration| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
        let alive = self.started.elapsed().saturating_sub(self.parked);
        ALIVE.fetch_add(nanos(alive), Ordering::Relaxed);
        BUSY.fetch_add(nanos(self.busy), Ordering::Relaxed);
        worker(false);
    }
}
