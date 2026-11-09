//! Builds a [`Skeleton`]: one pass over the first-parent chain, newest first, from the chain
//! the index stored when there is one (no commit record read), else by walking the records.
//! Both feed one tally, so they give the same skeleton.

use std::collections::{BTreeSet, HashMap};

use gitterrace_index::PathFilter;
use gitterrace_index::feed::{FileRow, InsightsSource, PathId, Rank};

use crate::languages::chain::{
    Fold, MARKS, MIN_GAP, Mark, Point, Skeleton, point, rows_newest_first, walk, wanted,
};

/// Tally state at one commit: its rank, net lines per slot and the added and removed sums.
pub(super) type Snapshot = (Rank, Vec<i64>, u64, u64);

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

    /// The tally's current state, saved to be subtracted from the final totals.
    pub(super) fn snapshot(&self, rank: Rank) -> Snapshot {
        (rank, self.net.clone(), self.added, self.removed)
    }
}

/// Where a kept position's id, time and offset come from: the stored chain carries them,
/// the walk reads them from the commit row afterwards.
type Kept = (usize, Snapshot, Option<(String, i64, i32)>);

/// What one pass over the chain leaves: the tally and its snapshots.
struct Pass<'a> {
    n: usize,
    gap: usize,
    want: BTreeSet<usize>,
    tally: Tally<'a>,
    fixed: Vec<Kept>,
    marks: Vec<Snapshot>,
    pos: usize,
}

impl<'a> Pass<'a> {
    fn new(n: usize, src: &'a dyn InsightsSource, filter: &'a PathFilter) -> Self {
        Self {
            n,
            gap: n.div_ceil(MARKS).max(MIN_GAP),
            want: wanted(n, None),
            tally: Tally::new(src, filter),
            fixed: Vec::new(),
            marks: Vec::new(),
            pos: n,
        }
    }

    /// The next commit, newer than every one before it in the pass. `at` gives the commit's
    /// id, time and offset when the stream carries them, asked only for a kept position.
    fn step(
        &mut self,
        r: Rank,
        files: &[FileRow],
        at: impl FnOnce() -> Option<(String, i64, i32)>,
    ) {
        self.pos -= 1;
        let pos = self.pos;
        if self.want.contains(&pos) {
            self.fixed.push((pos, self.tally.snapshot(r), at()));
        }
        if pos % self.gap == self.gap - 1 || pos == self.n - 1 {
            self.marks.push(self.tally.snapshot(r));
        }
        self.tally.apply(files);
    }
}

impl Skeleton {
    /// One pass over the chain, keeping the unranged view's positions and one mark per gap.
    /// The stored chain is read when the source has it, streamed and never held; otherwise
    /// the chain is walked, once. A stored chain cut short is dropped for the walk, and the
    /// source's failure makes the caller refuse the result anyway.
    pub(crate) fn build(src: &dyn InsightsSource, filter: &PathFilter) -> Self {
        if let Some(n) = src.chain_len() {
            let mut p = Pass::new(n, src, filter);
            let had = src.chain(&mut |s| {
                // A chain longer than its meta says is damage, which the source reports.
                if p.pos > 0 {
                    p.step(s.rank, &s.files, || Some((hex(&s.oid), s.time, s.offset)));
                }
            });
            if had && p.pos == 0 {
                return Self::from_pass(src, p);
            }
        }
        let newest = src
            .len()
            .checked_sub(1)
            .map(|i| Rank(u32::try_from(i).unwrap_or(u32::MAX)));
        // One walk: each step decodes a record and looks its parent up in the oid index, so
        // counting and then streaming the chain walked it twice (1.3M steps on linux).
        let chain: Vec<Rank> = walk(src, newest).collect();
        let mut p = Pass::new(chain.len(), src, filter);
        rows_newest_first(src, chain, |r, files| p.step(r, files, || None));
        Self::from_pass(src, p)
    }

    fn from_pass(src: &dyn InsightsSource, p: Pass<'_>) -> Self {
        let Pass {
            n,
            gap,
            tally,
            fixed,
            mut marks,
            ..
        } = p;
        let fold = Fold::new(&tally.names, &tally.net);
        let total = (&tally.net, tally.added, tally.removed);
        let after = |(_, newer, a, d): &Snapshot| {
            let row: Vec<i64> = total
                .0
                .iter()
                .enumerate()
                .map(|(s, v)| v - newer.get(s).copied().unwrap_or(0))
                .collect();
            (fold.apply(&row), total.1 - a, total.2 - d)
        };
        // Marks were met newest first; the mark of gap `g` is wanted at index `g`.
        marks.reverse();
        let marks = marks
            .iter()
            .map(|m| {
                let (lines, added, removed) = after(m);
                Mark {
                    rank: m.0,
                    lines,
                    added,
                    removed,
                }
            })
            .collect();
        let fixed = fixed
            .into_iter()
            .map(|(p, m, at)| {
                let (lines, added, removed) = after(&m);
                let kept = match at {
                    Some((id, time, offset)) => Point {
                        id,
                        time,
                        offset,
                        lines,
                        added,
                        removed,
                    },
                    None => point(src, m.0, lines, added, removed),
                };
                (p, kept)
            })
            .collect();
        Self {
            n,
            langs: fold.langs,
            gap,
            marks,
            fixed,
        }
    }
}

fn hex(oid: &[u8; 20]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(40);
    for b in oid {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    s
}
