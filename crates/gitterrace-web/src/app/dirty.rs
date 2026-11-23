//! The worktree's dirty answer, reused for a couple of seconds.

use std::{sync::Mutex, time::Instant};

use crate::app::DIRTY_TTL;

/// The worktree's dirty answer and when it was measured, reused for [`DIRTY_TTL`].
#[derive(Default)]
pub(crate) struct DirtyMemo(Mutex<Option<(Instant, bool)>>);

impl DirtyMemo {
    /// The answer when one inside the bound is at hand.
    pub(crate) fn get(&self) -> Option<bool> {
        let g = self.0.lock().ok()?;
        let (at, v) = (*g)?;
        (at.elapsed() < DIRTY_TTL).then_some(v)
    }

    pub(crate) fn put(&self, v: bool) {
        if let Ok(mut g) = self.0.lock() {
            *g = Some((Instant::now(), v));
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut g) = self.0.lock() {
            *g = None;
        }
    }
}
