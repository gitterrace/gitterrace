//! Builds a [`Skeleton`]: one pass over the first-parent chain, oldest first, walking the
//! records into one tally. The tally and a checkpoint every `gap` commits are kept as a
//! [`State`], so the next build for a newer tip reads only the commits since.

use std::collections::HashMap;
use std::sync::Arc;

use gitterrace_index::PathFilter;
use gitterrace_index::feed::{FileRow, InsightsSource, PathId, Rank};

use crate::languages::chain::state::{Check, State};
use crate::languages::chain::{
    Chain, Fold, MARKS, MIN_GAP, Mark, Skeleton, point, rows_oldest_first, unranged, walk, wanted,
};

/// Commits between marks for a chain of `n`: `MIN_GAP` doubled until at most `MARKS` marks
/// cover it. A power-of-two multiple, so a longer chain keeps every other checkpoint of a
/// shorter one's.
pub(super) const fn gap_for(n: usize) -> usize {
    let mut gap = MIN_GAP;
    while n.div_ceil(gap) > MARKS {
        gap *= 2;
    }
    gap
}

/// Net lines per language slot over the commits applied so far.
pub(super) struct Tally<'a> {
    src: &'a dyn InsightsSource,
    filter: &'a PathFilter,
    pub(super) names: Vec<String>,
    slot: HashMap<String, usize>,
    /// Language slot per path id, `None` when the path filter leaves it out.
    lang_of: HashMap<PathId, Option<usize>>,
    /// Language slot per language key ([`gitterrace_index::language_key`]): one language
    /// decision per extension, not per path.
    by_key: HashMap<(bool, String), usize>,
    pub(super) net: Vec<i64>,
    pub(super) added: u64,
    pub(super) removed: u64,
}

impl<'a> Tally<'a> {
    pub(super) fn new(src: &'a dyn InsightsSource, filter: &'a PathFilter) -> Self {
        Self {
            src,
            filter,
            names: Vec::new(),
            slot: HashMap::new(),
            lang_of: HashMap::new(),
            by_key: HashMap::new(),
            net: Vec::new(),
            added: 0,
            removed: 0,
        }
    }

    pub(super) fn apply(&mut self, files: &[FileRow]) {
        for f in files {
            let s = if let Some(s) = self.lang_of.get(&f.path) {
                *s
            } else {
                let s = self.slot_of(f.path);
                self.lang_of.insert(f.path, s);
                s
            };
            let Some(s) = s else {
                continue;
            };
            self.net[s] += i64::from(f.add) - i64::from(f.del);
            self.added += u64::from(f.add);
            self.removed += u64::from(f.del);
        }
    }

    fn slot_of(&mut self, id: PathId) -> Option<usize> {
        let path = self.src.path(id).unwrap_or_default();
        if self.filter.skips(&path) {
            return None;
        }
        let (ext, key) = gitterrace_index::language_key(&path);
        if let Some(s) = self.by_key.get(&(ext, key.to_owned())) {
            return Some(*s);
        }
        let l = gitterrace_index::language(&path);
        let s = if let Some(s) = self.slot.get(&l) {
            *s
        } else {
            self.names.push(l.clone());
            self.net.push(0);
            self.slot.insert(l, self.names.len() - 1);
            self.names.len() - 1
        };
        self.by_key.insert((ext, key.to_owned()), s);
        Some(s)
    }

    /// The tally as `state` left it, for a chain that goes on from its tip.
    fn restore(&mut self, state: &State) {
        self.names.clone_from(&state.names);
        self.slot = state.names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
        self.net.clone_from(&state.net);
        self.added = state.added;
        self.removed = state.removed;
    }

    fn check(&self, src: &dyn InsightsSource, rank: Rank) -> Check {
        Check {
            id: src.commit_id(rank).map(|s| s.to_string()).unwrap_or_default(),
            net: self.net.clone(),
            added: self.added,
            removed: self.removed,
        }
    }
}

impl Skeleton {
    /// The skeleton of the chain at the source's newest commit and the state to extend it
    /// from. The source's failure makes the caller refuse the result.
    pub(crate) fn build(src: &dyn InsightsSource, filter: &PathFilter) -> (Self, State) {
        Self::grow(src, filter, None).unwrap_or_else(|| (Self::empty(), State::empty()))
    }

    /// The same as [`Skeleton::build`] from `prev`, the state of an older tip on this chain,
    /// reading only the commits since. `None` when `prev` does not fit: its tip is off this
    /// chain or a checkpoint is gone from the source. The caller then builds from scratch.
    pub(crate) fn extend(
        src: &dyn InsightsSource,
        filter: &PathFilter,
        prev: &State,
    ) -> Option<(Self, State)> {
        Self::grow(src, filter, Some(prev))
    }

    fn empty() -> Self {
        Self { n: 0, langs: Vec::new(), gap: MIN_GAP, marks: Vec::new(), fixed: HashMap::new() }
    }

    fn grow(
        src: &dyn InsightsSource,
        filter: &PathFilter,
        base: Option<&State>,
    ) -> Option<(Self, State)> {
        let newest = src.len().checked_sub(1).map(|i| Rank(u32::try_from(i).unwrap_or(u32::MAX)));
        // Newest first down to the saved tip, or to the root. One walk: each step decodes a
        // record and looks its parent up.
        let mut fresh: Vec<Rank> = Vec::new();
        let mut found = base.is_none();
        for r in walk(src, newest) {
            if base.is_some_and(|b| src.commit_id(r).as_deref() == Some(b.tip.as_str())) {
                found = true;
                break;
            }
            fresh.push(r);
        }
        if !found {
            return None;
        }
        let base_n = base.map_or(0, |b| b.n);
        let n = base_n + fresh.len();
        let gap = gap_for(n);
        let mut tally = Tally::new(src, filter);
        let mut checks: Vec<(Rank, Check)> = Vec::new();
        if let Some(b) = base {
            if !gap.is_multiple_of(b.gap) {
                return None;
            }
            let every = gap / b.gap;
            for c in b.checks.iter().skip(every - 1).step_by(every) {
                checks.push((src.commit_of(&c.id)?, c.clone()));
            }
            tally.restore(b);
        }
        fresh.reverse();
        // The positions the unranged page reads, met on the way: their totals are folded once
        // the final ones are known. Those in the part below `base` are resolved from marks.
        let want = wanted(n, None);
        let mut snaps: Vec<Snap> = Vec::new();
        let mut count = base_n;
        rows_oldest_first(src, &fresh, |r, files| {
            tally.apply(files);
            if want.contains(&count) {
                snaps.push((count, r, tally.net.clone(), tally.added, tally.removed));
            }
            count += 1;
            if count.is_multiple_of(gap) {
                checks.push((r, tally.check(src, r)));
            }
        });
        Some(Self::assemble(src, filter, (n, gap), tally, checks, &snaps, newest))
    }

    /// The skeleton and state of a chain of `n` commits: `tally` after all of them and the
    /// `checks` every `gap` commits, with the ranks they sit at.
    fn assemble(
        src: &dyn InsightsSource,
        filter: &PathFilter,
        (n, gap): (usize, usize),
        tally: Tally<'_>,
        checks: Vec<(Rank, Check)>,
        snaps: &[Snap],
        newest: Option<Rank>,
    ) -> (Self, State) {
        let fold = Fold::new(&tally.names, &tally.net);
        let mark = |rank: Rank, net: &[i64], added: u64, removed: u64| {
            let mut row = net.to_vec();
            row.resize(tally.names.len(), 0);
            Mark { rank, lines: fold.apply(&row), added, removed }
        };
        let mut marks: Vec<Mark> =
            checks.iter().map(|(r, c)| mark(*r, &c.net, c.added, c.removed)).collect();
        // The last gap, when the chain ends inside it, is marked at the tip.
        if n % gap != 0
            && let Some(r) = newest
        {
            marks.push(mark(r, &tally.net, tally.added, tally.removed));
        }
        let fixed = snaps
            .iter()
            .map(|(pos, r, net, added, removed)| {
                let mut row = net.clone();
                row.resize(tally.names.len(), 0);
                (*pos, point(src, *r, fold.apply(&row), *added, *removed))
            })
            .collect();
        let mut sk = Self { n, langs: fold.langs, gap, marks, fixed };
        sk.resolve_missing(src, filter);
        let state = State {
            n,
            gap,
            tip: newest.and_then(|r| src.commit_id(r)).map(|s| s.to_string()).unwrap_or_default(),
            names: tally.names,
            net: tally.net,
            added: tally.added,
            removed: tally.removed,
            checks: checks.into_iter().map(|(_, c)| c).collect(),
        };
        (sk, state)
    }

    /// Resolves the unranged page's positions the pass did not meet, from the marks like any
    /// view's. None on a build from nothing.
    fn resolve_missing(&mut self, src: &dyn InsightsSource, filter: &PathFilter) {
        if self.n == 0 || self.fixed.len() >= wanted(self.n, None).len() {
            return;
        }
        let me = Arc::new(Self {
            n: self.n,
            langs: self.langs.clone(),
            gap: self.gap,
            marks: std::mem::take(&mut self.marks),
            fixed: std::mem::take(&mut self.fixed),
        });
        let Chain { extra, .. } = me.view(src, filter, unranged());
        if let Ok(mut back) = Arc::try_unwrap(me) {
            back.fixed.extend(extra);
            *self = back;
        }
    }
}

/// A position the pass met: its index, rank and the tally after it.
type Snap = (usize, Rank, Vec<i64>, u64, u64);
