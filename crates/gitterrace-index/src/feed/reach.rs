//! A view of a rev other than the indexed tip, over the same segments: the indexed commits
//! the rev reaches, as a rank bit set, plus the few it has that the set lacks (`fresh.rs`).
//!
//! The bit set comes from the stored parents alone. Canonical order puts parents before
//! children, so one sweep from the tip's rank down to 0 marks every ancestor, with no stack
//! and no git walk. Restricting the set's canonical order to an ancestor-closed subset gives
//! that subset's own canonical order, so dense ranks here equal the ranks a fresh index of
//! the rev would give its indexed part. Memory: one bit per indexed commit plus a u32 per 64.

use std::collections::BTreeMap;
use std::io;
use std::ops::Range;
use std::sync::Arc;

use crate::CommitSource;
use crate::feed::fresh::{self, Fresh, FreshRow, Walk, label_count};
use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, SourceError, Stream, Sums,
};

/// Commits per `rows_for` call while folding an aggregate, so one fold holds one chunk.
const CHUNK: usize = 256;

/// What a rev other than the indexed tip gets.
pub enum RevView<S> {
    Ready(crate::feed::Checked<Reach<S>>),
    /// The rev has more than the cap of commits the set lacks: reindex for it.
    NeedsReindex {
        read: usize,
    },
}

/// The view. Ranks are dense: the reached indexed commits first, in their set order, then the
/// unindexed ones.
pub struct Reach<S> {
    base: S,
    bits: Vec<u64>,
    /// Set bits before each word.
    before: Vec<u32>,
    inner: u32,
    fresh: Fresh,
}

/// Builds the view of `tip` (a commit id) over `base`, reading at most `cap` commits from git.
/// `rev` names the mailmap for identities the set lacks.
pub fn rev_view<S: InsightsSource>(
    base: S,
    src: &dyn CommitSource,
    rev: &str,
    tip: &str,
    cap: usize,
) -> io::Result<RevView<S>> {
    // Absence reads git, so an error must not pass for it.
    let (fresh, seeds) = match base.find_commit(tip).map_err(SourceError::io)? {
        Some(r) => (Fresh::default(), vec![r]),
        None => match fresh::walk(&base, src, rev, tip, cap)? {
            Walk::Done(f, met) => (f, met),
            Walk::TooFar(read) => return Ok(RevView::NeedsReindex { read }),
        },
    };
    let mut reach = Reach::new(base, &seeds);
    reach.fresh = fresh;
    reach.shift_fresh();
    Ok(RevView::Ready(crate::feed::Checked::new(Arc::new(reach))))
}

fn bit(bits: &[u64], r: u32) -> bool {
    bits.get(r as usize / 64)
        .is_some_and(|w| w >> (r % 64) & 1 == 1)
}

impl<S: InsightsSource> Reach<S> {
    fn new(base: S, seeds: &[Rank]) -> Self {
        let n = u32::try_from(base.len()).unwrap_or(u32::MAX);
        let mut bits = vec![0u64; (n as usize).div_ceil(64)];
        let mut top = None;
        for s in seeds.iter().filter(|s| s.0 < n) {
            bits[s.0 as usize / 64] |= 1 << (s.0 % 64);
            top = top.max(Some(s.0));
        }
        if let Some(top) = top {
            for r in (0..=top).rev() {
                if !bit(&bits, r) {
                    continue;
                }
                for p in base.parent_ids(Rank(r)) {
                    if let Some(pr) = base.commit_of(&p).filter(|pr| pr.0 < r) {
                        bits[pr.0 as usize / 64] |= 1 << (pr.0 % 64);
                    }
                }
            }
        }
        let mut before = Vec::with_capacity(bits.len());
        let mut acc = 0u32;
        for w in &bits {
            before.push(acc);
            acc += w.count_ones();
        }
        Self {
            base,
            bits,
            before,
            inner: acc,
            fresh: Fresh::default(),
        }
    }

    /// Fresh rows were ranked after the whole set, move them to follow the reached part.
    fn shift_fresh(&mut self) {
        let n = u32::try_from(self.base.len()).unwrap_or(u32::MAX);
        for f in &mut self.fresh.rows {
            f.row.rank = Rank(f.row.rank.0 - n + self.inner);
        }
    }

    /// The source the indexed part reads, for its unreadable check.
    pub const fn base(&self) -> &S {
        &self.base
    }

    /// Commits the view read from git.
    pub const fn fresh_len(&self) -> usize {
        self.fresh.rows.len()
    }

    fn dense(&self, base: Rank) -> Option<Rank> {
        if !bit(&self.bits, base.0) {
            return None;
        }
        let w = base.0 as usize / 64;
        let below = self.bits[w] & ((1u64 << (base.0 % 64)) - 1);
        Some(Rank(self.before[w] + below.count_ones()))
    }

    fn to_base(&self, dense: Rank) -> Option<Rank> {
        if dense.0 >= self.inner {
            return None;
        }
        let w = self.before.partition_point(|&b| b <= dense.0) - 1;
        let mut word = self.bits[w];
        for _ in 0..dense.0 - self.before[w] {
            word &= word - 1;
        }
        Some(Rank(
            u32::try_from(w * 64).unwrap_or(u32::MAX) + word.trailing_zeros(),
        ))
    }

    fn fresh_at(&self, r: Rank) -> Option<&FreshRow> {
        r.0.checked_sub(self.inner)
            .and_then(|i| self.fresh.rows.get(i as usize))
    }

    /// Every commit with its rows, chunked, ascending.
    fn each_row(&self, mut f: impl FnMut(&CommitRow, &[FileRow])) {
        let n = u32::try_from(self.len()).unwrap_or(u32::MAX);
        let mut at = 0;
        while at < n {
            let end = (at + CHUNK as u32).min(n);
            let commits: Vec<CommitRow> = self.commits_in(CommitRange::Ranks(at..end)).collect();
            let ranks = commits.iter().map(|c| c.rank).collect();
            for (c, (_, files)) in commits.iter().zip(self.rows_for(ranks)) {
                f(c, &files);
            }
            at = end;
        }
    }

    fn remap_hits(&self, s: Stream<'_, Hit>) -> Vec<Hit> {
        s.filter_map(|h| self.dense(h.rank).map(|rank| Hit { rank, ..h }))
            .collect()
    }
}

impl<S: InsightsSource> InsightsSource for Reach<S> {
    fn len(&self) -> usize {
        self.inner as usize + self.fresh.rows.len()
    }

    fn commit(&self, rank: Rank) -> Option<CommitRow> {
        if let Some(b) = self.to_base(rank) {
            return self.base.commit(b).map(|c| CommitRow { rank, ..c });
        }
        self.fresh_at(rank).map(|f| f.row)
    }

    fn commits_in(&self, range: CommitRange) -> Stream<'_, CommitRow> {
        let n = u32::try_from(self.len()).unwrap_or(u32::MAX);
        let (ranks, time) = match range {
            CommitRange::All => (0..n, None),
            CommitRange::Ranks(r) => (r.start.min(n)..r.end.min(n), None),
            CommitRange::Time(t) => (0..n, Some(t)),
        };
        Box::new(
            ranks
                .filter_map(move |r| self.commit(Rank(r)))
                .filter(move |c| time.as_ref().is_none_or(|t| t.contains(&c.time))),
        )
    }

    fn rows_for(&self, commits: Vec<Rank>) -> Stream<'_, (Rank, Vec<FileRow>)> {
        Box::new(commits.into_iter().filter_map(move |r| {
            if let Some(b) = self.to_base(r) {
                return self.base.rows_for(vec![b]).next().map(|(_, f)| (r, f));
            }
            self.fresh_at(r).map(|f| (r, f.files.clone()))
        }))
    }

    fn commit_id(&self, rank: Rank) -> Option<Arc<str>> {
        self.to_base(rank).map_or_else(
            || self.fresh_at(rank).map(|f| Arc::clone(&f.id)),
            |b| self.base.commit_id(b),
        )
    }

    fn parent_ids(&self, rank: Rank) -> Vec<Arc<str>> {
        self.to_base(rank).map_or_else(
            || {
                self.fresh_at(rank)
                    .map(|f| f.parents.clone())
                    .unwrap_or_default()
            },
            |b| self.base.parent_ids(b),
        )
    }

    fn find_commit(&self, id: &str) -> Result<Option<Rank>, SourceError> {
        if let Some(&i) = self.fresh.ids.get(id) {
            return Ok(Some(Rank(
                self.inner + u32::try_from(i).unwrap_or(u32::MAX),
            )));
        }
        Ok(self.base.find_commit(id)?.and_then(|b| self.dense(b)))
    }

    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow> {
        let mut m: BTreeMap<(i64, IdentId), (u64, Sums)> = BTreeMap::new();
        for c in self.commits_in(CommitRange::All) {
            let day = crate::feed::local_day_of(c.time, c.offset);
            if c.merge || !days.contains(&day) {
                continue;
            }
            let e = m.entry((day, c.ident)).or_default();
            e.0 += 1;
            e.1.merge(&c.sums);
        }
        Box::new(
            m.into_iter()
                .map(|((day, ident), (commits, sums))| AuthorDayRow {
                    day,
                    ident,
                    commits,
                    sums,
                }),
        )
    }

    fn agg_path_author(&self, prefix: &str) -> Stream<'_, PathAuthorRow> {
        let prefix = prefix.trim_matches('/');
        let mut keep: BTreeMap<PathId, bool> = BTreeMap::new();
        let mut m: BTreeMap<(PathId, IdentId), (u64, Sums)> = BTreeMap::new();
        self.each_row(|c, files| {
            if c.merge {
                return;
            }
            for f in files {
                let ok = *keep.entry(f.path).or_insert_with(|| {
                    prefix.is_empty()
                        || self
                            .path(f.path)
                            .is_some_and(|p| crate::insights::under(&p, prefix))
                });
                if ok {
                    let e = m.entry((f.path, c.ident)).or_default();
                    e.0 += 1;
                    e.1.add_row(f);
                }
            }
        });
        Box::new(
            m.into_iter()
                .map(|((path, ident), (touches, sums))| PathAuthorRow {
                    path,
                    ident,
                    touches,
                    sums,
                }),
        )
    }

    fn postings(&self, key: Posting) -> Stream<'_, Hit> {
        let mut out = self.remap_hits(self.base.postings(key));
        for FreshRow { row: c, files, .. } in &self.fresh.rows {
            match key {
                Posting::Author(a) if c.ident == a => out.push(Hit {
                    rank: c.rank,
                    row: 0,
                }),
                Posting::Path(p) if !c.merge => {
                    for (i, f) in files.iter().enumerate() {
                        if f.path == p {
                            out.push(Hit {
                                rank: c.rank,
                                row: u32::try_from(i).unwrap_or(u32::MAX),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        Box::new(out.into_iter())
    }

    fn rename_edges(&self) -> Stream<'_, RenameEdge> {
        let mut out: Vec<RenameEdge> = self
            .base
            .rename_edges()
            .filter_map(|e| self.dense(e.rank).map(|rank| RenameEdge { rank, ..e }))
            .collect();
        for FreshRow { row: c, files, .. } in &self.fresh.rows {
            for (i, f) in files.iter().enumerate() {
                if let Some(old) = f.old {
                    out.push(RenameEdge {
                        rank: c.rank,
                        row: u32::try_from(i).unwrap_or(u32::MAX),
                        old,
                        new: f.path,
                    });
                }
            }
        }
        Box::new(out.into_iter())
    }

    fn path(&self, id: PathId) -> Option<Arc<str>> {
        (id.0 as usize)
            .checked_sub(self.base.paths())
            .map_or_else(|| self.base.path(id), |i| self.fresh.paths.get(i).cloned())
    }

    fn find_path(&self, path: &str) -> Result<Option<PathId>, SourceError> {
        if let Some(p) = self.base.find_path(path)? {
            return Ok(Some(p));
        }
        let Some(i) = self.fresh.paths.iter().position(|p| &**p == path) else {
            return Ok(None);
        };
        Ok(u32::try_from(self.base.paths() + i).ok().map(PathId))
    }

    fn paths(&self) -> usize {
        self.base.paths() + self.fresh.paths.len()
    }

    fn ident(&self, id: IdentId) -> Option<(Arc<str>, Arc<str>)> {
        (id.0 as usize).checked_sub(self.base.idents()).map_or_else(
            || self.base.ident(id),
            |i| self.fresh.idents.get(i).cloned(),
        )
    }

    fn idents(&self) -> usize {
        self.base.idents() + self.fresh.idents.len()
    }

    fn label(&self, id: LabelId) -> Option<Arc<str>> {
        self.base.label(id).or_else(|| {
            (id.0 as usize)
                .checked_sub(label_count(&self.base))
                .and_then(|i| self.fresh.labels.get(i).cloned())
        })
    }

    fn find_label(&self, name: &str) -> Result<Option<LabelId>, SourceError> {
        if let Some(l) = self.base.find_label(name)? {
            return Ok(Some(l));
        }
        let Some(i) = self.fresh.labels.iter().position(|l| &**l == name) else {
            return Ok(None);
        };
        Ok(u32::try_from(label_count(&self.base) + i).ok().map(LabelId))
    }

    fn failure(&self) -> Option<SourceError> {
        self.base.failure()
    }
}

#[cfg(test)]
#[path = "reach_tests.rs"]
mod tests;
