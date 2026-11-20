//! The many-path touch read without a tip file: one pass over the kept commits' file rows.

use rustc_hash::FxHashMap as FastMap;

use crate::feed::{CommitRange, CommitRow, PathId, Rank};
use crate::insights::Insights;
use crate::insights::files::TouchRaw;

impl Insights<'_> {
    /// [`Insights::touches_many`] for many paths: one ascending pass over the kept commits
    /// file rows instead of a postings lookup per path, which is tens of thousands of lookups
    /// for the head files of a large tree. State is the lists themselves plus the head and
    /// alias ids, never a table per commit.
    pub(super) fn touches_scan(
        &self,
        paths: &[&str],
        keep: impl Fn(&CommitRow) -> bool,
    ) -> Vec<Vec<(TouchRaw, CommitRow)>> {
        let src = self.source();
        let follow = self.follow();
        // Every name a head file had, to the head files it can stand for.
        let mut names: FastMap<PathId, Vec<(usize, PathId)>> = FastMap::default();
        for (i, p) in paths.iter().enumerate() {
            let Some(head) = src.path_id(p) else {
                continue;
            };
            names.entry(head).or_default().push((i, head));
            for &old in follow.names.get(&head).into_iter().flatten() {
                names.entry(old).or_default().push((i, head));
            }
        }
        let kept: Vec<CommitRow> =
            src.commits_in(CommitRange::All).filter(|c| !c.merge && keep(c)).collect();
        let ranks: Vec<Rank> = kept.iter().map(|c| c.rank).collect();
        let mut commits = kept.into_iter();
        let mut out: Vec<Vec<(TouchRaw, CommitRow)>> = vec![Vec::new(); paths.len()];
        for (rank, rows) in src.rows_for(ranks) {
            let Some(c) = commits.by_ref().find(|c| c.rank == rank) else {
                break;
            };
            for (i, r) in rows.iter().enumerate() {
                let Some(heads) = names.get(&r.path) else {
                    continue;
                };
                let row = u32::try_from(i).unwrap_or(u32::MAX);
                for &(at, head) in heads {
                    if follow.canonical(r.path, rank, row) != head {
                        continue;
                    }
                    let l = self.row_lines_unfiltered(r);
                    let touch = TouchRaw { commit: rank, row, path: r.path, lines: l.add + l.del };
                    out[at].push((touch, c));
                }
            }
        }
        // The order of the per-path lookup: newest commit first, rows ascending.
        for list in &mut out {
            list.sort_unstable_by(|x, y| {
                y.0.commit.cmp(&x.0.commit).then_with(|| x.0.row.cmp(&y.0.row))
            });
            list.dedup_by_key(|t| (t.0.commit, t.0.row));
        }
        out
    }
}
