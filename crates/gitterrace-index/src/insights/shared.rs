//! The derived tables of recent unfiltered sources, shared by the pages of one tip, see
//! [`super::Insights::shared_source`].

use std::sync::{Arc, Mutex};

use gitterrace_git::IgnoreSet;

use crate::feed::InsightsSource;
use crate::insights::{Derived, Options};

/// Derived tables kept for this many (key, options) pairs: the unfiltered pages of a few tips.
const SHARED_SETS: usize = 4;

/// One repository's derived tables, see [`super::Insights::shared_source`].
///
/// Each served repository holds its own, so another one's pages never evict or release
/// its tables.
#[derive(Default)]
pub struct SharedTables {
    sets: Mutex<Vec<SharedEntry>>,
    /// Held while a set is built: one build per set, see [`SharedTables::tables`].
    building: Mutex<()>,
}

struct SharedEntry {
    key: String,
    opts: Options,
    /// Address and length of what the tables index into: a hit must be the same set.
    at: (usize, usize),
    d: Arc<Derived>,
}

impl SharedTables {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops every set, returning whether any was held. Callers holding an `Insights` keep
    /// their own `Arc`; the next page builds again.
    pub fn clear(&self) -> bool {
        self.sets.lock().is_ok_and(|mut g| {
            let held = !g.is_empty();
            g.clear();
            held
        })
    }

    /// Sets held now (a test hook for [`SharedTables::clear`]).
    #[cfg(any(test, feature = "oracle"))]
    pub fn len(&self) -> usize {
        self.sets.lock().map_or(0, |g| g.len())
    }

    /// True when no set is held.
    #[cfg(any(test, feature = "oracle"))]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(super) fn tables(
        &self,
        src: &dyn InsightsSource,
        opts: &Options,
        key: &str,
        at: (usize, usize),
        ignore: impl FnOnce() -> Option<IgnoreSet>,
    ) -> Arc<Derived> {
        let find = || {
            self.sets.lock().ok().and_then(|g| {
                g.iter()
                    .find(|e| e.key == key && e.opts == *opts && e.at == at)
                    .map(|e| Arc::clone(&e.d))
            })
        };
        if let Some(d) = find() {
            return d;
        }
        // One build per set: a page arriving while the warm-up builds waits for its tables
        // instead of building a second copy beside it.
        let _one = self.building.lock();
        if let Some(d) = find() {
            return d;
        }
        let d = Arc::new(Derived::build(src, opts, ignore()));
        // Tables built while a read failed hold a short people table. They serve this caller,
        // whose `Checked::run` refuses the page, and are never kept: a later page over blocks
        // that do read would index into them.
        if src.failure().is_some() {
            return d;
        }
        if let Ok(mut g) = self.sets.lock() {
            g.retain(|e| !(e.key == key && e.opts == *opts));
            if g.len() >= SHARED_SETS {
                g.remove(0);
            }
            g.push(SharedEntry { key: key.to_owned(), opts: opts.clone(), at, d: Arc::clone(&d) });
        }
        d
    }
}
