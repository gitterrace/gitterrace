mod reports;

use std::collections::HashSet;
use std::ops::RangeInclusive;

use crate::feed::{
    CommitRange, CommitRow, InsightsSource, PathId, Posting, Rank, RenameEdge, local_day_of,
};
use crate::insights::fasthash::FastMap;
use crate::insights::{Insights, Period};

/// Most files in one commit that still say something about coupling; bigger commits are
/// sweeps (formatting, renames, vendoring).
/// Not configurable: it defines what coupling means, so a setting would change the numbers.
pub(super) const COUPLING_MAX_FILES: usize = 30;
/// Above this many paths a request scans the file rows once instead of looking each path up
/// in the postings: a lookup reads its blocks, a scan reads every row once.
/// Not configurable: a query plan threshold, the answer is the same either way.
const PER_PATH_MAX: usize = 64;
pub(super) const SOLO_SHARE: f64 = 0.8;
pub(super) const STALE_SECS: i64 = 365 * crate::insights::DAY;

/// One change to a file, followed to its canonical (head) name.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TouchRaw {
    pub commit: Rank,
    /// The file row index inside the commit.
    pub row: u32,
    /// The file row inside the commit, which holds the name at that time.
    pub path: PathId,
    pub lines: u32,
}

/// The rename aliases of the whole history, replayed newest first as the old in-memory follow
/// did: a name maps to the name its file has at the newest commit. Only rename rows change an
/// alias, so the log is sized by renames, not by history.
#[derive(Default)]
pub(crate) struct Follow {
    /// Per path id, every alias it took, in replay order: (rank, row, canonical id). A row
    /// sees the aliases set by newer commits and by earlier rows of its own commit.
    log: FastMap<PathId, Vec<(Rank, u32, PathId)>>,
    /// Per canonical id, the other ids that ever aliased to it.
    names: FastMap<PathId, Vec<PathId>>,
}

impl Follow {
    /// Replays `edges` newest commit first, rows ascending, merges left out.
    pub(crate) fn build(src: &dyn InsightsSource) -> Self {
        let mut edges: Vec<RenameEdge> = src.rename_edges().collect();
        edges.sort_by(|x, y| y.rank.cmp(&x.rank).then_with(|| x.row.cmp(&y.row)));
        let mut alias: FastMap<PathId, PathId> = FastMap::default();
        let mut out = Self::default();
        let mut merge: Option<(Rank, bool)> = None;
        for e in edges {
            let is_merge = match merge {
                Some((r, m)) if r == e.rank => m,
                _ => {
                    let m = src.commit(e.rank).is_some_and(|c| c.merge);
                    merge = Some((e.rank, m));
                    m
                }
            };
            if is_merge {
                continue;
            }
            let canon = alias.get(&e.new).copied().unwrap_or(e.new);
            alias.insert(e.old, canon);
            out.log
                .entry(e.old)
                .or_default()
                .push((e.rank, e.row, canon));
            let n = out.names.entry(canon).or_default();
            if !n.contains(&e.old) {
                n.push(e.old);
            }
        }
        out
    }

    /// Canonical id of `path` as seen by row `row` of commit `rank`.
    fn canonical(&self, path: PathId, rank: Rank, row: u32) -> PathId {
        // The log is in replay order: rank descending, rows ascending within a rank. The
        // aliases a row sees are a prefix of it, so its last entry is found by bisection
        // (hits of a busy path times a long rename log made the linear walk quadratic).
        self.log
            .get(&path)
            .and_then(|l| {
                let seen = l.partition_point(|(r, w, _)| *r > rank || (*r == rank && *w < row));
                seen.checked_sub(1).map(|i| l[i])
            })
            .map_or(path, |e| e.2)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Touch {
    pub commit: String,
    pub time: i64,
    pub author: String,
    pub lines: u32,
    /// Name the file had in that commit.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Hotspot {
    pub path: String,
    pub language: String,
    /// Lines now.
    pub lines: u32,
    /// Commits that touched the file in the period.
    pub changes: usize,
    /// Added plus removed lines in the period.
    pub lines_changed: u64,
    pub authors: usize,
    pub last_change: i64,
    /// `changes * sqrt(lines)`.
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HotspotReport {
    /// Sorted by score. The same rows feed the scatter plot (x changes, y lines, size authors).
    pub files: Vec<Hotspot>,
    pub total_changes: usize,
    /// Share of all changes that hit the top 10% of files.
    pub concentration: f64,
    pub top_tenth: usize,
    pub single_author_files: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Owner {
    pub name: String,
    /// Share of the knowledge, 0..=1.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DirOwnership {
    /// `""` for the whole repository.
    pub path: String,
    pub lines: u64,
    pub files: usize,
    pub owners: Vec<Owner>,
    /// People who together hold at least half of the knowledge.
    pub bus_factor: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OwnershipReport {
    pub overall: DirOwnership,
    pub dirs: Vec<DirOwnership>,
    /// Lines in files where one person made at least 80% of the changes.
    pub solo_lines: u64,
    /// Lines whose main owner has no commit in the 12 months before the newest commit.
    pub stale_lines: u64,
    /// Lines in files with no change in the period, not counted above.
    pub untracked_lines: u64,
    pub dirs_with_bus_factor_one: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Coupling {
    pub a: String,
    pub b: String,
    /// Commits that changed both.
    pub together: usize,
    pub a_changes: usize,
    pub b_changes: usize,
    /// `together / min(a_changes, b_changes)`.
    pub degree: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgeBucket {
    pub year: i64,
    pub lines: u64,
    pub files: usize,
}

/// Lines now by the year their file was last touched. This is a blame-free approximation:
/// a file counts fully toward its newest change, so old code in a recently edited file looks
/// younger than it is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgeReport {
    pub buckets: Vec<AgeBucket>,
    pub total_lines: u64,
}

impl Insights<'_> {
    pub(crate) fn build_follow(&self) -> Follow {
        Follow::build(self.source())
    }

    /// Follow hits of `path` (the name at the newest commit), newest commit first, rows
    /// ascending: the order the old full replay produced.
    fn hits_of(&self, path: &str) -> Vec<(Rank, u32, PathId)> {
        let src = self.source();
        let Some(head) = src.path_id(path) else {
            return Vec::new();
        };
        let follow = self.follow();
        let mut hits: Vec<(Rank, u32, PathId)> = Vec::new();
        let mut names = vec![head];
        names.extend(follow.names.get(&head).into_iter().flatten().copied());
        for p in names {
            for h in src.postings(Posting::Path(p)) {
                if follow.canonical(p, h.rank, h.row) == head {
                    hits.push((h.rank, h.row, p));
                }
            }
        }
        hits.sort_unstable_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.cmp(&y.1)));
        hits.dedup_by_key(|h| (h.0, h.1));
        hits
    }

    /// Touches of every path in `paths`, one list per path. Each commit is read once for all
    /// paths and the rows in one ascending pass, so the cost is the touches, not files x
    /// commits. `keep` drops commits before their rows are read.
    fn touches_many(
        &self,
        paths: &[&str],
        keep: impl Fn(&CommitRow) -> bool,
    ) -> Vec<Vec<(TouchRaw, CommitRow)>> {
        if paths.len() > PER_PATH_MAX {
            return self.touches_scan(paths, keep);
        }
        let src = self.source();
        let hits: Vec<Vec<(Rank, u32, PathId)>> = paths.iter().map(|p| self.hits_of(p)).collect();
        let mut ranks: Vec<Rank> = hits.iter().flatten().map(|h| h.0).collect();
        ranks.sort_unstable();
        ranks.dedup();
        // One ascending pass over the commit table: random lookups per touched commit cost a
        // block-cache probe and a decode each, hundreds of thousands on a large history.
        let commits: FastMap<Rank, CommitRow> = src
            .commits_in(CommitRange::All)
            .filter(|c| ranks.binary_search(&c.rank).is_ok() && keep(c))
            .map(|c| (c.rank, c))
            .collect();
        ranks.retain(|r| commits.contains_key(r));
        let want: HashSet<(Rank, u32)> = hits
            .iter()
            .flatten()
            .filter(|h| commits.contains_key(&h.0))
            .map(|h| (h.0, h.1))
            .collect();
        let mut lines: FastMap<(Rank, u32), u32> = FastMap::default();
        for (rank, rows) in src.rows_for(ranks) {
            for (i, r) in rows.iter().enumerate() {
                let row = u32::try_from(i).unwrap_or(u32::MAX);
                if want.contains(&(rank, row)) {
                    let l = self.row_lines_unfiltered(r);
                    lines.insert((rank, row), l.add + l.del);
                }
            }
        }
        hits.into_iter()
            .map(|list| {
                list.into_iter()
                    .filter_map(|(commit, row, path)| {
                        let c = *commits.get(&commit)?;
                        let lines = lines.get(&(commit, row)).copied().unwrap_or(0);
                        Some((
                            TouchRaw {
                                commit,
                                row,
                                path,
                                lines,
                            },
                            c,
                        ))
                    })
                    .collect()
            })
            .collect()
    }

    /// [`Insights::touches_many`] for many paths: one ascending pass over the kept commits
    /// file rows instead of a postings lookup per path, which is tens of thousands of lookups
    /// for the head files of a large tree. State is the lists themselves plus the head and
    /// alias ids, never a table per commit.
    fn touches_scan(
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
        let kept: Vec<CommitRow> = src
            .commits_in(CommitRange::All)
            .filter(|c| !c.merge && keep(c))
            .collect();
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
                    let touch = TouchRaw {
                        commit: rank,
                        row,
                        path: r.path,
                        lines: l.add + l.del,
                    };
                    out[at].push((touch, c));
                }
            }
        }
        // The order of the per-path lookup: newest commit first, rows ascending.
        for list in &mut out {
            list.sort_unstable_by(|x, y| {
                y.0.commit
                    .cmp(&x.0.commit)
                    .then_with(|| x.0.row.cmp(&y.0.row))
            });
            list.dedup_by_key(|t| (t.0.commit, t.0.row));
        }
        out
    }

    /// Every non-merge touch of `path`, through renames, newest first.
    fn touches_of(&self, path: &str) -> Vec<(TouchRaw, CommitRow)> {
        self.touches_many(&[path], |_| true)
            .pop()
            .unwrap_or_default()
    }

    /// Touches of each head path by counted commits whose local day lies in `days` and that
    /// `chips` keeps, newest first. The local day is the one rule [`crate::Scope::admits`]
    /// uses: a commit at the edge of a period counts on the day its author saw.
    pub(super) fn touches_in(
        &self,
        paths: &[&str],
        days: &RangeInclusive<i64>,
        chips: &impl Fn(&CommitRow) -> bool,
    ) -> Vec<Vec<(TouchRaw, CommitRow)>> {
        self.touches_many(paths, |c| {
            days.contains(&local_day_of(c.time, c.offset)) && self.counted_row(c) && chips(c)
        })
    }

    /// The local days `period` covers, ending on this history's newest local day: the days
    /// [`crate::CountContext::days`] gives the same period over the same history.
    pub(super) fn period_days(&self, period: Period) -> RangeInclusive<i64> {
        if period.days().is_none() {
            return i64::MIN..=i64::MAX;
        }
        self.in_period(period, |c| c.days().clone())
            .unwrap_or(RangeInclusive::new(1, 0))
    }

    /// (rank, row) of every follow touch of `path`, newest first: the old follow table's
    /// entry, for cross-checks against the segment postings.
    #[cfg(test)]
    pub(crate) fn follow_hits(&self, path: &str) -> Vec<(u32, u32)> {
        self.touches_of(path)
            .iter()
            .map(|(t, _)| (t.commit.0, t.row))
            .collect()
    }

    /// Full history of a file as it is named at the newest commit, following renames.
    /// Includes bots and every non-merge commit, newest first.
    pub fn file_history(&self, path: &str) -> Vec<Touch> {
        let src = self.source();
        let mut out: Vec<(Rank, Touch)> = self
            .touches_of(path)
            .into_iter()
            .map(|(t, c)| {
                (
                    t.commit,
                    Touch {
                        commit: src
                            .commit_id(t.commit)
                            .map(|s| s.to_string())
                            .unwrap_or_default(),
                        time: c.time,
                        author: self.d.people.list[self.person_of(c.ident)].name.clone(),
                        lines: t.lines,
                        path: src.path(t.path).map(|s| s.to_string()).unwrap_or_default(),
                    },
                )
            })
            .collect();
        // Ties on time go to the newer rank: never the storage order.
        out.sort_by_key(|(r, t)| (std::cmp::Reverse(t.time), std::cmp::Reverse(*r)));
        out.into_iter().map(|(_, t)| t).collect()
    }
}

#[cfg(test)]
mod tests;
