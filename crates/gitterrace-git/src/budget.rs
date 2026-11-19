//! A worker limit another thread may change while a parallel diff runs.

use std::sync::{Arc, Condvar, Mutex};

/// How many of a run's workers may take work now.
///
/// Cloned handles share one limit: the index child reads the budget the server sends and
/// sets it here, and the workers of [`crate::Repo::numstat_stream`] follow it from their next
/// item on. A worker past the limit parks between items, so lowering it never cuts a diff
/// short and never changes a result. Zero parks every worker: the run pauses until the limit
/// rises.
#[derive(Clone, Debug)]
pub struct Budget(Arc<(Mutex<usize>, Condvar)>);

impl Budget {
    #[must_use]
    pub fn new(live: usize) -> Self {
        Self(Arc::new((Mutex::new(live), Condvar::new())))
    }

    /// Sets the limit and wakes the parked workers to look at it.
    pub fn set(&self, live: usize) {
        if let Ok(mut s) = self.0.0.lock() {
            *s = live;
        }
        self.0.1.notify_all();
    }

    #[must_use]
    pub fn get(&self) -> usize {
        self.0.0.lock().map_or(0, |s| *s)
    }

    /// Waits until worker `id` (counted from 0) is inside the limit or `stop` holds. Returns
    /// whether the worker may go on. One budget serves many runs, so a run that ends wakes its
    /// parked workers with [`Budget::wake`] instead of closing the budget.
    pub(crate) fn wait_turn(&self, id: usize, stop: impl Fn() -> bool) -> bool {
        let Ok(mut s) = self.0.0.lock() else {
            return false;
        };
        while id >= *s && !stop() {
            let Ok(g) = self.0.1.wait(s) else {
                return false;
            };
            s = g;
        }
        !stop()
    }

    /// Wakes every parked worker to look at its stop flag again. Taken under the lock, so a
    /// worker between its check and its wait cannot miss it.
    pub(crate) fn wake(&self) {
        let held = self.0.0.lock();
        self.0.1.notify_all();
        drop(held);
    }
}
