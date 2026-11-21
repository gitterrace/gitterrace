mod reports;
mod scan;

use rustc_hash::FxHashMap as FastMap;
use std::collections::HashSet;

use crate::feed::{CommitRange, CommitRow, InsightsSource, PathId, Posting, Rank, RenameEdge};
use crate::insights::{Counted, Insights};

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

/// One stored tip touch: commit rank, row in it, tip name, raw and whitespace-blind churn.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tipped {
    pub rank: u32,
    pub row: u32,
    pub path: u32,
    pub raw: u32,
    pub code: u32,
}

/// The stored tip touches (`crate::tip`) grouped by tip name, newest commit first and rows
/// ascending within one, plus the commit columns: what every per-file section slices, built
/// once per set and dropped with the shared tables. About 20 B per touch (28 MB on rust).
pub(crate) struct TipIndex {
    /// Per tip path id, the start of its touches in `touches`; one past the end last.
    start: Vec<u32>,
    touches: Vec<Tipped>,
    pub cols: Vec<CommitRow>,
}

impl TipIndex {
    /// `None` when the source has no tip file for this set.
    pub(crate) fn build(src: &dyn InsightsSource) -> Option<Self> {
        if !src.has_tip() {
            return None;
        }
        let cols: Vec<CommitRow> = src.commits_in(CommitRange::All).collect();
        let mut count = vec![0u32; src.paths() + 1];
        let mut flat: Vec<Tipped> = Vec::new();
        let had = src.tip_touches(&mut |rank, rows| {
            for (i, t) in rows.iter().enumerate() {
                if let Some(n) = count.get_mut(t.path.0 as usize) {
                    *n += 1;
                }
                flat.push(Tipped {
                    rank: rank.0,
                    row: u32::try_from(i).unwrap_or(u32::MAX),
                    path: t.path.0,
                    raw: t.raw,
                    code: t.code,
                });
            }
        });
        if !had {
            return None;
        }
        // Counting sort by path, ranks descending inside one: walk the flat list (ascending
        // rank, rows ascending) backwards per commit.
        let mut start = Vec::with_capacity(count.len() + 1);
        let mut at = 0u32;
        for n in &count {
            start.push(at);
            at += n;
        }
        start.push(at);
        let mut fill = start.clone();
        let mut touches = vec![Tipped { rank: 0, row: 0, path: 0, raw: 0, code: 0 }; flat.len()];
        for group in flat.chunk_by(|a, b| a.rank == b.rank).rev() {
            for t in group {
                if let Some(f) = fill.get_mut(t.path as usize) {
                    touches[*f as usize] = *t;
                    *f += 1;
                }
            }
        }
        Some(Self { start, touches, cols })
    }

    pub(crate) fn of(&self, id: PathId) -> Option<&[Tipped]> {
        let i = id.0 as usize;
        let (a, b) = (*self.start.get(i)?, *self.start.get(i + 1)?);
        self.touches.get(a as usize..b as usize)
    }
}

/// One change to a file, followed to its canonical (head) name.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TouchRaw {
    pub commit: Rank,
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
    names: FastMap<PathId, Vec<PathId>>,
}

impl Follow {
    /// Replays `edges` newest commit first, rows ascending, merges left out.
    pub(crate) fn build(src: &dyn InsightsSource) -> Self {
        let edges: Vec<RenameEdge> = src.rename_edges().collect();
        Self::of_edges(edges, |r| src.commit(r).is_some_and(|c| c.merge))
    }

    /// [`Follow::build`] over `edges`, `merge` telling the merge commits apart.
    pub(crate) fn of_edges(mut edges: Vec<RenameEdge>, merge_at: impl Fn(Rank) -> bool) -> Self {
        edges.sort_by(|x, y| y.rank.cmp(&x.rank).then_with(|| x.row.cmp(&y.row)));
        let mut alias: FastMap<PathId, PathId> = FastMap::default();
        let mut out = Self::default();
        let mut merge: Option<(Rank, bool)> = None;
        for e in edges {
            let is_merge = match merge {
                Some((r, m)) if r == e.rank => m,
                _ => {
                    let m = merge_at(e.rank);
                    merge = Some((e.rank, m));
                    m
                }
            };
            if is_merge {
                continue;
            }
            let canon = alias.get(&e.new).copied().unwrap_or(e.new);
            alias.insert(e.old, canon);
            out.log.entry(e.old).or_default().push((e.rank, e.row, canon));
            let n = out.names.entry(canon).or_default();
            if !n.contains(&e.old) {
                n.push(e.old);
            }
        }
        out
    }

    /// Canonical id of `path` as seen by row `row` of commit `rank`.
    pub(crate) fn canonical(&self, path: PathId, rank: Rank, row: u32) -> PathId {
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
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Hotspot {
    pub path: String,
    pub language: String,
    pub lines: u32,
    pub changes: usize,
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

/// Lines of one language in the head tree.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LanguageLines {
    pub files: usize,
    pub language: String,
    pub lines: u64,
}

/// Lines per language in the head tree, the numbers the Languages page lists for now.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LanguageReport {
    pub files: usize,
    /// Most lines first, ties by name.
    pub languages: Vec<LanguageLines>,
    pub total_lines: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Owner {
    pub name: String,
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
        #[cfg(test)]
        LISTED.with(|n| n.set(n.get() + paths.len()));
        if paths.len() > PER_PATH_MAX {
            if let Some(tip) = self.tip_index() {
                return self.touches_tip(tip, paths, keep);
            }
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
                        Some((TouchRaw { commit, row, path, lines }, c))
                    })
                    .collect()
            })
            .collect()
    }

    /// [`Insights::touches_scan`] from the shared [`TipIndex`]: each touch already carries
    /// the name its file has at the indexed tip, so this slices each head path's list and
    /// follows no rename. The same touches in the same order; `TouchRaw::path` is the tip
    /// name, which no caller of the many-path read shows.
    fn touches_tip(
        &self,
        tip: &TipIndex,
        paths: &[&str],
        keep: impl Fn(&CommitRow) -> bool,
    ) -> Vec<Vec<(TouchRaw, CommitRow)>> {
        let raw = self.opts.count_whitespace;
        // A commit is kept or not for every path: decide it once.
        let mut kept: Vec<Option<bool>> = vec![None; tip.cols.len()];
        paths
            .iter()
            .map(|p| {
                let Some(list) = self.head_id(p).and_then(|id| tip.of(id)) else {
                    return Vec::new();
                };
                list.iter()
                    .filter_map(|t| {
                        let c = tip.cols.get(t.rank as usize)?;
                        let k = kept.get_mut(t.rank as usize)?;
                        if !*k.get_or_insert_with(|| keep(c)) {
                            return None;
                        }
                        let touch = TouchRaw {
                            commit: Rank(t.rank),
                            row: t.row,
                            path: PathId(t.path),
                            lines: if raw { t.raw } else { t.code },
                        };
                        Some((touch, *c))
                    })
                    .collect()
            })
            .collect()
    }

    /// Calls `each(i, lines, person, commit)` for every touch of `paths[i]` by the commits `c`
    /// counts, newest first per path, the touches [`Insights::touches_in`] lists, `person`
    /// being [`Insights::person_of`] its author. Read straight from the shared tip index when
    /// there is one, so no list per path is built; `each` returns false to skip the rest of
    /// that path. A caller that needs no commit field never reads the commit columns.
    pub(super) fn visit_touches_in(
        &self,
        paths: &[&str],
        c: &Counted<'_>,
        mut each: impl FnMut(usize, u32, usize, &CommitRow) -> bool,
    ) {
        let tip = (paths.len() > PER_PATH_MAX).then(|| self.tip_index()).flatten();
        let Some(tip) = tip else {
            for (i, list) in self.touches_in(paths, c).into_iter().enumerate() {
                for (t, row) in list {
                    if !each(i, t.lines, self.person_of(row.ident), &row) {
                        break;
                    }
                }
            }
            return;
        };
        let raw = self.opts.count_whitespace;
        // Per rank, its author's person when `c` counts it, else none: 4 bytes a commit, so
        // the touch loop reads this, not a commit row, for each touch.
        let skip = u32::MAX;
        let who: Vec<u32> = tip
            .cols
            .iter()
            .map(|row| {
                if c.contains_columns(row) {
                    u32::try_from(self.person_of(row.ident)).unwrap_or(skip)
                } else {
                    skip
                }
            })
            .collect();
        for (i, p) in paths.iter().enumerate() {
            let Some(list) = self.head_id(p).and_then(|id| tip.of(id)) else {
                continue;
            };
            for t in list {
                let Some(&person) = who.get(t.rank as usize) else {
                    continue;
                };
                if person == skip {
                    continue;
                }
                let Some(row) = tip.cols.get(t.rank as usize) else {
                    continue;
                };
                if !each(i, if raw { t.raw } else { t.code }, person as usize, row) {
                    break;
                }
            }
        }
    }

    /// Every non-merge touch of `path`, through renames, newest first: the cross-check's
    /// view, bots included.
    #[cfg(test)]
    fn touches_of(&self, path: &str) -> Vec<(TouchRaw, CommitRow)> {
        self.touches_many(&[path], |_| true).pop().unwrap_or_default()
    }

    /// Touches of each head path by the commits `c` counts, newest first. The path selection
    /// is the caller's: it picks the head files, and a touch is a row of one of them.
    pub(super) fn touches_in(
        &self,
        paths: &[&str],
        c: &Counted<'_>,
    ) -> Vec<Vec<(TouchRaw, CommitRow)>> {
        self.touches_many(paths, |r| c.contains_columns(r))
    }

    /// (rank, row) of every follow touch of `path`, newest first: the old follow table's
    /// entry, for cross-checks against the segment postings.
    #[cfg(test)]
    pub(crate) fn follow_hits(&self, path: &str) -> Vec<(u32, u32)> {
        self.touches_of(path).iter().map(|(t, _)| (t.commit.0, t.row)).collect()
    }

    /// Full history of a file as it is named at the newest commit, following renames: every
    /// commit that counts under the options ([`Insights::counted_row`]: no merges, bots as
    /// the options say), newest first. The hotspots row of the file counts the same commits.
    pub fn file_history(&self, path: &str) -> Vec<Touch> {
        let src = self.source();
        let mut out: Vec<(Rank, Touch)> = self
            .touches_many(&[path], |c| self.counted_row(c))
            .pop()
            .unwrap_or_default()
            .into_iter()
            .map(|(t, c)| {
                (
                    t.commit,
                    Touch {
                        commit: src.commit_id(t.commit).map(|s| s.to_string()).unwrap_or_default(),
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
thread_local! {
    /// Paths whose touches were built as lists, for the counted-work guard: the per-file
    /// pages over the tip index visit the touches in place and build none.
    pub(crate) static LISTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
