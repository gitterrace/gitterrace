//! Every per-request cache a repository keeps, in one place, so an idle server drops them all.
//!
//! [`Caches::clear_all`] names each field in one destructuring pattern: a cache added here
//! without being cleared there does not compile. The rendered-page memo is not one of them,
//! it is the byte budget the settings name.

use std::sync::{Arc, Mutex};

use gitterrace_git::LastCommit;
use gitterrace_index::CommitRecord;

use crate::bounded::Bounded;

/// One selected commit's records, filled once by whichever caller gets there first.
pub(crate) type RecordSlot = Arc<Mutex<Option<Arc<Vec<CommitRecord>>>>>;
/// A commit id and the index generation it was read at.
pub(crate) type HeadGen = (String, u64);
/// One commit and its ignore files as `(path, text)`.
pub(crate) type IgnoreFiles = (String, Arc<Vec<(String, String)>>);
/// Overview totals for one (commit, index generation, options).
pub(crate) type TotalsEntry = (String, gitterrace_index::PeriodStats);
/// Whether the set behind a served source met a store error, and which.
pub(crate) type Unreadable = Arc<dyn Fn() -> Option<String> + Send + Sync>;
/// Commit subjects by full id.
pub(crate) type Subjects = std::collections::HashMap<String, String>;

pub(crate) struct Caches {
    /// Record sets by (commit, generation), oldest first. The map lock is held only to find a
    /// slot; the parse runs under that slot's own lock, so one cold commit never stalls pages
    /// on another.
    pub(crate) records: Mutex<Vec<(HeadGen, RecordSlot)>>,
    /// The insights source of one (commit, generation), so every page shares its derived
    /// tables.
    pub(crate) source: Mutex<Option<(HeadGen, crate::insights::Src)>>,
    /// The same source as `source` holds when it reads the segment files, typed, so a store
    /// error it met turns into a rebuild.
    pub(crate) disk: Mutex<Option<Unreadable>>,
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
    pub(crate) subjects: Mutex<Option<(String, Arc<Subjects>)>>,
    /// The languages skeleton by (head, index generation). Flat in history length.
    pub(crate) chain: Mutex<Option<(HeadGen, Arc<crate::languages::Skeleton>)>>,
    pub(crate) head_lines: Mutex<Option<(String, Arc<Vec<gitterrace_index::HeadFile>>)>>,
}

impl Caches {
    pub(crate) fn new(small: usize) -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            source: Mutex::new(None),
            disk: Mutex::new(None),
            far: Mutex::new(None),
            totals: Mutex::new(None),
            ignores: Mutex::new(None),
            file_lists: Mutex::new(Bounded::new(small)),
            last_commits: Mutex::new(Bounded::new(small)),
            path_logs: Mutex::new(Bounded::new(small)),
            subjects: Mutex::new(None),
            chain: Mutex::new(None),
            head_lines: Mutex::new(None),
        }
    }

    /// Empties every cache, returning whether any held something. A page in flight keeps its
    /// own `Arc`, so it is never cut short; the next page builds again.
    pub(crate) fn clear_all(&self) -> bool {
        let Self {
            records,
            source,
            disk,
            far,
            totals,
            ignores,
            file_lists,
            last_commits,
            path_logs,
            subjects,
            chain,
            head_lines,
        } = self;
        [
            vec_held(records),
            take(source),
            take(disk),
            take(far),
            take(totals),
            take(ignores),
            bounded_held(file_lists),
            bounded_held(last_commits),
            bounded_held(path_logs),
            take(subjects),
            take(chain),
            take(head_lines),
        ]
        .into_iter()
        .fold(false, |any, held| any | held)
    }

    /// Sets the byte budget of the bounded caches.
    pub(crate) fn set_budget(&self, small: usize) {
        if let Ok(mut g) = self.file_lists.lock() {
            g.set_budget(small);
        }
        if let Ok(mut g) = self.last_commits.lock() {
            g.set_budget(small);
        }
        if let Ok(mut g) = self.path_logs.lock() {
            g.set_budget(small);
        }
    }
}

fn take<T>(m: &Mutex<Option<T>>) -> bool {
    m.lock().is_ok_and(|mut g| g.take().is_some())
}

fn vec_held<T>(m: &Mutex<Vec<T>>) -> bool {
    m.lock().is_ok_and(|mut g| {
        let held = !g.is_empty();
        g.clear();
        held
    })
}

fn bounded_held<V: Clone + crate::bounded::Weight>(m: &Mutex<Bounded<V>>) -> bool {
    m.lock().is_ok_and(|mut g| {
        let held = !g.is_empty();
        g.clear();
        held
    })
}
