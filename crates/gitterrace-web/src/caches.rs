//! Every per-request cache a repository keeps, in one place, so an idle server drops them all.
//!
//! [`Caches::clear_all`] names each field in one destructuring pattern: a cache added here
//! without being cleared there does not compile. The rendered-page memo is not one of them,
//! it is the byte budget the settings name.

use std::sync::{Arc, Mutex};

use gitterrace_config::{Cache, Config};
use gitterrace_git::LastCommit;

use crate::bounded::Bounded;

/// A commit id and the index generation it was read at.
pub(crate) type HeadGen = (String, u64);
/// One commit and its ignore files as `(path, text)`.
pub(crate) type IgnoreFiles = (String, Arc<Vec<(String, String)>>);
/// Overview totals for one (commit, index generation, options).
pub(crate) type TotalsEntry = (String, OverviewTotals);

/// The overview's two populations of one history: every commit, and the counted totals.
#[derive(Debug, Clone)]
pub(crate) struct OverviewTotals {
    pub(crate) every: gitterrace_index::EveryTotals,
    pub(crate) counted: gitterrace_index::PeriodStats,
}

pub(crate) struct Caches {
    /// The insights source of one (commit, generation), so every page shares its derived
    /// tables.
    pub(crate) source: Mutex<Option<(HeadGen, crate::insights::CheckedSrc)>>,
    /// Commits whose insights are past the reach of the segment set, with how many commits
    /// were read to find out, keyed by (commit, generation).
    pub(crate) far: Mutex<Option<(HeadGen, usize)>>,
    pub(crate) totals: Mutex<Option<TotalsEntry>>,
    /// Ignore files of one commit. Building the rules from them is cheap; finding them walks
    /// the whole tree.
    pub(crate) ignores: Mutex<Option<IgnoreFiles>>,
    pub(crate) file_lists: Mutex<Bounded<Arc<Vec<String>>>>,
    pub(crate) last_commits: Mutex<Bounded<Arc<Vec<LastCommit>>>>,
    /// Ids of the commits `git log -- path` lists, by (tip, path, index generation).
    pub(crate) path_logs: Mutex<Bounded<Arc<crate::app::PathLog>>>,
    /// The languages skeleton by (head, index generation). Flat in history length.
    /// Comparisons read from git by (base id, head id). Ids never change, so no entry goes
    /// stale; only a successful read is kept.
    pub(crate) compares: Mutex<Bounded<Arc<gitterrace_index::Comparison>>>,
    pub(crate) chain: Mutex<Option<(HeadGen, Arc<crate::languages::Skeleton>)>>,
    pub(crate) head_lines: Mutex<Option<(String, Arc<Vec<gitterrace_index::HeadFile>>)>>,
}

impl Caches {
    pub(crate) fn new(cfg: &Config) -> Self {
        Self {
            source: Mutex::new(None),
            far: Mutex::new(None),
            totals: Mutex::new(None),
            ignores: Mutex::new(None),
            file_lists: Mutex::new(Bounded::new(cfg.cache_bytes(Cache::FileLists))),
            last_commits: Mutex::new(Bounded::new(cfg.cache_bytes(Cache::LastCommits))),
            path_logs: Mutex::new(Bounded::new(cfg.cache_bytes(Cache::PathLogs))),
            compares: Mutex::new(Bounded::new(cfg.cache_bytes(Cache::Compares))),
            chain: Mutex::new(None),
            head_lines: Mutex::new(None),
        }
    }

    /// Empties every cache, returning whether any held something. A page in flight keeps its
    /// own `Arc`, so it is never cut short; the next page builds again.
    pub(crate) fn clear_all(&self) -> bool {
        let Self {
            source,
            far,
            totals,
            ignores,
            file_lists,
            last_commits,
            path_logs,
            compares,
            chain,
            head_lines,
        } = self;
        [
            take(source),
            take(far),
            take(totals),
            take(ignores),
            bounded_held(file_lists),
            bounded_held(last_commits),
            bounded_held(path_logs),
            bounded_held(compares),
            take(chain),
            take(head_lines),
        ]
        .into_iter()
        .fold(false, |any, held| any | held)
    }

    /// Sets the byte budget of the bounded caches.
    pub(crate) fn set_budget(&self, cfg: &Config) {
        if let Ok(mut g) = self.file_lists.lock() {
            g.set_budget(cfg.cache_bytes(Cache::FileLists));
        }
        if let Ok(mut g) = self.last_commits.lock() {
            g.set_budget(cfg.cache_bytes(Cache::LastCommits));
        }
        if let Ok(mut g) = self.path_logs.lock() {
            g.set_budget(cfg.cache_bytes(Cache::PathLogs));
        }
        if let Ok(mut g) = self.compares.lock() {
            g.set_budget(cfg.cache_bytes(Cache::Compares));
        }
    }
}

fn take<T>(m: &Mutex<Option<T>>) -> bool {
    m.lock().is_ok_and(|mut g| g.take().is_some())
}

fn bounded_held<V: Clone + crate::bounded::Weight>(m: &Mutex<Bounded<V>>) -> bool {
    m.lock().is_ok_and(|mut g| {
        let held = !g.is_empty();
        g.clear();
        held
    })
}
