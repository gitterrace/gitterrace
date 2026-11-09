//! The derived tables of recent unfiltered sources, shared by the pages of one tip, see
//! [`super::Insights::shared_source`].

use std::sync::{Arc, Mutex};

use gitterrace_git::IgnoreSet;

use crate::feed::InsightsSource;
use crate::insights::{Derived, Options};

/// Derived tables kept for this many (key, options) pairs: the unfiltered pages of a few tips.
const SHARED_SETS: usize = 4;

/// Derived tables of recent unfiltered sources, see [`Insights::shared_source`].
pub(super) static SHARED: Mutex<Vec<SharedEntry>> = Mutex::new(Vec::new());

pub(super) struct SharedEntry {
    pub(super) key: String,
    pub(super) opts: Options,
    /// Address and length of what the tables index into: a hit must be the same set.
    pub(super) at: (usize, usize),
    pub(super) d: Arc<Derived>,
}

pub(super) fn tables(
    src: &dyn InsightsSource,
    opts: &Options,
    key: &str,
    at: (usize, usize),
    ignore: impl FnOnce() -> Option<IgnoreSet>,
) -> Arc<Derived> {
    let hit = SHARED.lock().ok().and_then(|g| {
        g.iter()
            .find(|e| e.key == key && e.opts == *opts && e.at == at)
            .map(|e| Arc::clone(&e.d))
    });
    hit.unwrap_or_else(|| {
        let d = Arc::new(Derived::build(src, opts, ignore()));
        if let Ok(mut g) = SHARED.lock() {
            g.retain(|e| !(e.key == key && e.opts == *opts));
            if g.len() >= SHARED_SETS {
                g.remove(0);
            }
            g.push(SharedEntry {
                key: key.to_owned(),
                opts: opts.clone(),
                at,
                d: Arc::clone(&d),
            });
        }
        d
    })
}
