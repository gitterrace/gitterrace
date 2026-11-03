use std::collections::{BTreeMap, HashSet};

use gitterrace_git::civil_from_days;

use crate::feed::{CommitRange, CommitRow, InsightsSource, PathId, Posting, Rank, RenameEdge};
use crate::insights::fasthash::FastMap;
use crate::insights::{DAY, HeadFile, Insights, Period};

/// Most files in one commit that still say something about coupling; bigger commits are
/// sweeps (formatting, renames, vendoring).
const COUPLING_MAX_FILES: usize = 30;
/// Above this many paths a request scans the file rows once instead of looking each path up
/// in the postings: a lookup reads its blocks, a scan reads every row once.
const PER_PATH_MAX: usize = 64;
const SOLO_SHARE: f64 = 0.8;
const STALE_SECS: i64 = 365 * 86_400;

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
        let Some(head) = src.find_path(path) else {
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
            let Some(head) = src.find_path(p) else {
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

    /// Touches of each head path in the period by counted commits, newest first.
    fn touches_in(&self, paths: &[&str], from: i64, to: i64) -> Vec<Vec<(TouchRaw, CommitRow)>> {
        self.touches_many(paths, |c| {
            c.time >= from && c.time < to && self.counted_row(c)
        })
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

    /// Hotspots among `files` (the head tree) for the period.
    pub fn hotspots(&self, files: &[HeadFile], period: Period) -> HotspotReport {
        let (from, to) = period.window(self.now);
        let mut rows = Vec::new();
        let shown: Vec<&HeadFile> = files
            .iter()
            .filter(|f| f.lines != 0 && !self.skips_head(&f.path))
            .collect();
        let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
        for (f, touches) in shown.into_iter().zip(self.touches_in(&names, from, to)) {
            let mut count = 0;
            let mut churn = 0u64;
            let mut last = i64::MIN;
            let mut who = HashSet::new();
            for (t, c) in touches {
                count += 1;
                churn += u64::from(t.lines);
                last = last.max(c.time);
                who.insert(self.person_of(c.ident));
            }
            if count == 0 {
                continue;
            }
            rows.push(Hotspot {
                path: f.path.clone(),
                language: super::language(&f.path),
                lines: f.lines,
                changes: count,
                lines_changed: churn,
                authors: who.len(),
                last_change: last,
                score: count as f64 * f64::from(f.lines).sqrt(),
            });
        }
        rows.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.path.cmp(&b.path))
        });
        let total: usize = rows.iter().map(|r| r.changes).sum();
        let tenth = rows.len().div_ceil(10).max(1);
        let mut by_changes: Vec<usize> = rows.iter().map(|r| r.changes).collect();
        by_changes.sort_unstable_by(|a, b| b.cmp(a));
        let top: usize = by_changes.iter().take(tenth).sum();
        HotspotReport {
            concentration: if total == 0 {
                0.0
            } else {
                top as f64 / total as f64
            },
            top_tenth: tenth,
            single_author_files: rows.iter().filter(|r| r.authors == 1).count(),
            total_changes: total,
            files: rows,
        }
    }

    /// Lines changed per person for each head file in the window, by counted commits.
    ///
    /// Over all history a file whose name never took part in a rename reads the path x author
    /// aggregate: every row under that name is its own, so the sums are exact without a pass
    /// over the file rows. Renamed files and windows go through the touches.
    fn owner_lines(&self, files: &[&HeadFile], from: i64, to: i64) -> Vec<BTreeMap<usize, u64>> {
        let src = self.source();
        let follow = self.follow();
        // The aggregate holds every commit: usable only when the window drops none.
        let whole = from == i64::MIN && self.d.newest < to;
        let plain = |f: &HeadFile| -> Option<PathId> {
            let id = src.find_path(&f.path)?;
            (!follow.log.contains_key(&id) && !follow.names.contains_key(&id)).then_some(id)
        };
        let ids: Vec<Option<PathId>> = files
            .iter()
            .map(|f| if whole { plain(f) } else { None })
            .collect();
        let mut out: Vec<BTreeMap<usize, u64>> = vec![BTreeMap::new(); files.len()];
        // Ordered by person (most commits first), so a tie for top owner and the float sums
        // come out the same on every run; a HashMap made "Stale lines" vary.
        let at: FastMap<PathId, usize> = ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| id.map(|id| (id, i)))
            .collect();
        if !at.is_empty() {
            let bots = self.opts.include_bots;
            for row in src.agg_path_author("") {
                let Some(&i) = at.get(&row.path) else {
                    continue;
                };
                let p = self.person_of(row.ident);
                if !bots && self.d.people.list[p].bot {
                    continue;
                }
                let s = &row.sums;
                let lines = if self.opts.count_whitespace {
                    s.add + s.del
                } else {
                    s.code_add + s.code_del
                };
                *out[i].entry(p).or_default() += lines;
            }
        }
        let rest: Vec<usize> = (0..files.len()).filter(|&i| ids[i].is_none()).collect();
        let names: Vec<&str> = rest.iter().map(|&i| &*files[i].path).collect();
        for (&i, touches) in rest.iter().zip(self.touches_in(&names, from, to)) {
            for (t, c) in touches {
                *out[i].entry(self.person_of(c.ident)).or_default() += u64::from(t.lines);
            }
        }
        // A person whose rows sum to zero lines still held the file in the touches; drop
        // nobody, the touches kept zero entries too.
        out
    }

    /// Knowledge share per file, weighted by its size now, summed per directory `depth` levels
    /// deep (`depth` 1 groups by top-level directory; root files group under `""`).
    pub fn ownership(&self, files: &[HeadFile], period: Period, depth: usize) -> OwnershipReport {
        let (from, to) = period.window(self.now);
        let mut dirs: BTreeMap<String, Acc> = BTreeMap::new();
        let mut all = Acc::default();
        let (mut solo, mut stale, mut untracked) = (0u64, 0u64, 0u64);
        let shown: Vec<&HeadFile> = files
            .iter()
            .filter(|f| f.lines != 0 && !self.skips_head(&f.path))
            .collect();
        let per_file = self.owner_lines(&shown, from, to);
        for (f, who) in shown.into_iter().zip(per_file) {
            let changed: u64 = who.values().sum();
            if changed == 0 {
                untracked += u64::from(f.lines);
                continue;
            }
            let parts: Vec<&str> = f.path.split('/').collect();
            let key = if parts.len() > 1 {
                parts[..depth.min(parts.len() - 1)].join("/")
            } else {
                String::new()
            };
            let d = dirs.entry(key).or_default();
            let (mut top_p, mut top_w) = (0, 0.0);
            for (p, c) in who {
                let w = f64::from(f.lines) * c as f64 / changed as f64;
                *d.w.entry(p).or_default() += w;
                *all.w.entry(p).or_default() += w;
                if w > top_w {
                    top_w = w;
                    top_p = p;
                }
            }
            d.lines += u64::from(f.lines);
            d.files += 1;
            all.lines += u64::from(f.lines);
            all.files += 1;
            if top_w / f64::from(f.lines) >= SOLO_SHARE {
                solo += u64::from(f.lines);
            }
            if self.d.last_seen[top_p] < self.now.saturating_sub(STALE_SECS) {
                stale += u64::from(f.lines);
            }
        }
        let finish = |path: String, a: Acc| -> DirOwnership {
            let mut owners: Vec<(usize, f64)> = a.w.into_iter().collect();
            owners.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
            let total: f64 = owners
                .iter()
                .map(|o| o.1)
                .sum::<f64>()
                .max(f64::MIN_POSITIVE);
            let bus = super::bus_factor(owners.iter().map(|o| o.1));
            DirOwnership {
                path,
                lines: a.lines,
                files: a.files,
                bus_factor: bus,
                owners: owners
                    .into_iter()
                    .map(|(p, w)| Owner {
                        name: self.d.people.list[p].name.clone(),
                        share: w / total,
                    })
                    .collect(),
            }
        };
        let mut dirs: Vec<DirOwnership> = dirs.into_iter().map(|(k, a)| finish(k, a)).collect();
        dirs.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.path.cmp(&b.path)));
        OwnershipReport {
            dirs_with_bus_factor_one: dirs
                .iter()
                .filter(|d| d.bus_factor == 1 && !d.owners.is_empty())
                .count(),
            overall: finish(String::new(), all),
            dirs,
            solo_lines: solo,
            stale_lines: stale,
            untracked_lines: untracked,
        }
    }

    /// Pairs of head files that changed in the same commits, strongest first. Pairs need at
    /// least `min_together` shared commits. Commits touching more than 30 files are skipped.
    pub fn coupling(
        &self,
        files: &[HeadFile],
        period: Period,
        min_together: usize,
        limit: usize,
    ) -> Vec<Coupling> {
        let (from, to) = period.window(self.now);
        let mut head: Vec<&str> = files
            .iter()
            .filter(|f| !self.skips_head(&f.path))
            .map(|f| &*f.path)
            .collect();
        head.sort_unstable();
        head.dedup();
        // Flat (commit, file) pairs sorted by commit replace a map of vectors: one allocation
        // and a sequential pass. State is sized by the touches of head files in the window.
        let mut touches: Vec<(u32, u32)> = Vec::new();
        let mut changes = vec![0_usize; head.len()];
        for (i, list) in self.touches_in(&head, from, to).into_iter().enumerate() {
            let id = u32::try_from(i).unwrap_or(u32::MAX);
            for (t, _) in list {
                touches.push((t.commit.0, id));
                changes[i] += 1;
            }
        }
        touches.sort_unstable();
        // Pairs are keyed by the two file numbers, a < b because each group is sorted.
        let mut pairs: FastMap<u64, usize> = FastMap::default();
        for group in touches.chunk_by(|x, y| x.0 == y.0) {
            if group.len() < 2 || group.len() > COUPLING_MAX_FILES {
                continue;
            }
            for (x, a) in group.iter().enumerate() {
                for b in &group[x + 1..] {
                    *pairs
                        .entry((u64::from(a.1) << 32) | u64::from(b.1))
                        .or_default() += 1;
                }
            }
        }
        // Cheap tuples first, strings only for the pairs that are shown.
        let mut kept: Vec<(u32, u32, usize)> = pairs
            .into_iter()
            .filter(|(_, n)| *n >= min_together)
            .map(|(key, together)| ((key >> 32) as u32, (key & 0xffff_ffff) as u32, together))
            .collect();
        let degree = |&(a, b, together): &(u32, u32, usize)| {
            together as f64 / changes[a as usize].min(changes[b as usize]) as f64
        };
        let order = |x: &(u32, u32, usize), y: &(u32, u32, usize)| {
            y.2.cmp(&x.2)
                .then_with(|| degree(y).total_cmp(&degree(x)))
                .then_with(|| head[x.0 as usize].cmp(head[y.0 as usize]))
                .then_with(|| head[x.1 as usize].cmp(head[y.1 as usize]))
        };
        if kept.len() > limit {
            if limit > 0 {
                kept.select_nth_unstable_by(limit - 1, order);
            }
            kept.truncate(limit);
        }
        kept.sort_by(order);
        kept.into_iter()
            .map(|t| {
                let (a, b, together) = t;
                let (ac, bc) = (changes[a as usize], changes[b as usize]);
                Coupling {
                    a: head[a as usize].to_owned(),
                    b: head[b as usize].to_owned(),
                    together,
                    a_changes: ac,
                    b_changes: bc,
                    degree: degree(&t),
                }
            })
            .collect()
    }

    /// Lines now grouped by the year of the file's last change. See [`AgeReport`].
    pub fn code_age(&self, files: &[HeadFile]) -> AgeReport {
        let mut years: BTreeMap<i64, (u64, usize)> = BTreeMap::new();
        let mut total = 0;
        let shown: Vec<&HeadFile> = files
            .iter()
            .filter(|f| f.lines != 0 && !self.skips_head(&f.path))
            .collect();
        let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
        let all = self.touches_in(&names, i64::MIN, i64::MAX);
        for (f, touches) in shown.into_iter().zip(all) {
            let Some((_, c)) = touches.into_iter().max_by_key(|t| t.0.commit) else {
                continue;
            };
            let day = c.time.saturating_add(i64::from(c.offset)).div_euclid(DAY);
            let e = years.entry(civil_from_days(day).0).or_default();
            e.0 += u64::from(f.lines);
            e.1 += 1;
            total += u64::from(f.lines);
        }
        AgeReport {
            buckets: years
                .into_iter()
                .map(|(year, (lines, files))| AgeBucket { year, lines, files })
                .collect(),
            total_lines: total,
        }
    }
}

#[derive(Default)]
struct Acc {
    lines: u64,
    files: usize,
    w: BTreeMap<usize, f64>,
}

#[cfg(test)]
mod tests {
    use super::super::HeadFile;
    use crate::insights::fixtures::{file, rec};
    use crate::{FileStat, Insights, Options, Period};

    fn head(files: &[(&str, u32)]) -> Vec<HeadFile> {
        files
            .iter()
            .map(|(p, n)| HeadFile {
                path: (*p).into(),
                lines: *n,
            })
            .collect()
    }

    fn renamed(old: &str, new: &str) -> FileStat {
        FileStat {
            old: Some(old.into()),
            ..file(new, 0, 0, 10)
        }
    }

    fn history() -> Vec<crate::CommitRecord> {
        vec![
            rec(
                "a",
                "Ann",
                0,
                vec![file("old/x.rs", 10, 0, 10), file("keep.rs", 4, 0, 4)],
            ),
            rec("b", "Bob", 400, vec![file("old/x.rs", 2, 1, 11)]),
            rec(
                "c",
                "Ann",
                401,
                vec![renamed("old/x.rs", "new/x.rs"), file("keep.rs", 1, 1, 4)],
            ),
            rec(
                "d",
                "Ann",
                402,
                vec![file("new/x.rs", 3, 0, 14), file("keep.rs", 1, 0, 5)],
            ),
        ]
    }

    #[test]
    fn ownership_is_the_same_on_every_build() {
        // Ann and Bob tie on every file; Ann is stale, Bob is not. A hash-ordered pick of the
        // top owner flipped "stale" between runs.
        let mut recs = Vec::new();
        for n in 0..40 {
            let path = format!("f{n}.rs");
            recs.push(rec(&format!("a{n}"), "Ann", 0, vec![file(&path, 5, 0, 5)]));
            recs.push(rec(
                &format!("b{n}"),
                "Bob",
                900,
                vec![file(&path, 0, 5, 5)],
            ));
        }
        let files: Vec<(String, u32)> = (0..40).map(|n| (format!("f{n}.rs"), 5)).collect();
        let files: Vec<(&str, u32)> = files.iter().map(|(p, l)| (p.as_str(), *l)).collect();
        let render = || {
            let ins = Insights::new(&recs, Options::default(), None);
            let r = ins.ownership(&head(&files), Period::All, 1);
            format!("{} {} {:?}", r.stale_lines, r.solo_lines, r.overall)
        };
        let first = render();
        for _ in 0..20 {
            assert_eq!(render(), first);
        }
    }

    #[test]
    fn hotspots_follow_renames() {
        let recs = history();
        let ins = Insights::new(&recs, Options::default(), None);
        let rep = ins.hotspots(&head(&[("new/x.rs", 14), ("keep.rs", 5)]), Period::All);
        let x = rep
            .files
            .iter()
            .find(|f| f.path == "new/x.rs")
            .map(|f| (f.changes, f.authors));
        assert_eq!(x, Some((4, 2)));
        assert_eq!(rep.files[0].path, "new/x.rs");
        assert!(4.0f64.mul_add(-14f64.sqrt(), rep.files[0].score).abs() < 1e-9);
        assert_eq!(rep.total_changes, 7);
        assert_eq!(rep.single_author_files, 1);
        let h = ins.file_history("new/x.rs");
        assert_eq!(h.len(), 4);
        assert_eq!(h[0].commit, "d");
    }

    #[test]
    fn ownership_and_bus_factor() {
        let recs = history();
        let ins = Insights::new(&recs, Options::default(), None);
        let o = ins.ownership(&head(&[("new/x.rs", 14), ("keep.rs", 5)]), Period::All, 1);
        // new/x.rs: Ann 10+0+3 = 13 changed lines, Bob 3. keep.rs: Ann only.
        let nx = o
            .dirs
            .iter()
            .find(|d| d.path == "new")
            .map(|d| d.owners[0].name.clone());
        assert_eq!(nx.as_deref(), Some("Ann"));
        assert_eq!(o.overall.bus_factor, 1);
        let root = o
            .dirs
            .iter()
            .find(|d| d.path.is_empty())
            .map(|d| d.bus_factor);
        assert_eq!(root, Some(1));
        let total: f64 = o.overall.owners.iter().map(|x| x.share).sum();
        assert!((total - 1.0).abs() < 1e-9);
        // Ann made 13 of the 16 changed lines of new/x.rs (81%), so both files are solo.
        assert_eq!(o.solo_lines, 19);
        assert_eq!(o.stale_lines, 0);
    }

    #[test]
    fn coupling_and_age() {
        let recs = history();
        let ins = Insights::new(&recs, Options::default(), None);
        let hf = head(&[("new/x.rs", 14), ("keep.rs", 5)]);
        let c = ins.coupling(&hf, Period::All, 2, 10);
        assert_eq!(c.len(), 1);
        assert_eq!(
            (c[0].together, c[0].a.as_str(), c[0].b.as_str()),
            (3, "keep.rs", "new/x.rs")
        );
        assert!((c[0].degree - 1.0).abs() < 1e-9);
        let age = ins.code_age(&hf);
        assert_eq!(age.total_lines, 19);
        assert_eq!(age.buckets.len(), 1);
        assert_eq!(age.buckets[0].year, 2025);
    }

    #[test]
    fn bots_are_excluded_by_default() {
        let recs = vec![
            rec("a", "Ann", 0, vec![file("a.rs", 5, 0, 5)]),
            rec("b", "dependabot[bot]", 1, vec![file("a.rs", 5, 0, 10)]),
        ];
        let hf = head(&[("a.rs", 10)]);
        let ins = Insights::new(&recs, Options::default(), None);
        assert_eq!(ins.hotspots(&hf, Period::All).files[0].changes, 1);
        let with = Insights::new(
            &recs,
            Options {
                include_bots: true,
                ..Options::default()
            },
            None,
        );
        assert_eq!(with.hotspots(&hf, Period::All).files[0].changes, 2);
    }

    /// The one-pass scan over many head paths answers exactly as a postings lookup per path,
    /// through renames, a name reused after it moved, and a file renamed twice.
    #[test]
    fn the_many_path_scan_equals_a_lookup_per_path() {
        let mut recs = history();
        recs.push(rec("e", "Bob", 500, vec![file("old/x.rs", 7, 0, 7)]));
        recs.push(rec("f", "Ann", 501, vec![renamed("new/x.rs", "z.rs")]));
        recs.push(rec("g", "Bob", 502, vec![file("z.rs", 1, 1, 14)]));
        // Padding so the scan path is taken.
        let mut names: Vec<String> = (0..super::PER_PATH_MAX + 5)
            .map(|n| format!("pad/{n}.rs"))
            .collect();
        for (i, n) in names.iter().enumerate() {
            recs.push(rec(&format!("p{i}"), "Cy", 600, vec![file(n, 1, 0, 1)]));
        }
        names.extend(["z.rs", "old/x.rs", "new/x.rs", "keep.rs", "missing.rs"].map(String::from));
        let ins = Insights::new(&recs, Options::default(), None);
        let paths: Vec<&str> = names.iter().map(String::as_str).collect();
        let scan = ins.touches_scan(&paths, |_| true);
        for (i, p) in paths.iter().enumerate() {
            let one = ins.touches_many(&[p], |_| true).pop().unwrap_or_default();
            let key = |l: &[(super::TouchRaw, crate::feed::CommitRow)]| {
                l.iter()
                    .map(|(t, c)| (t.commit, t.row, t.path, t.lines, c.rank))
                    .collect::<Vec<_>>()
            };
            assert_eq!(key(&scan[i]), key(&one), "{p}");
        }
    }

    /// A source that counts postings lookups, forwarding everything else.
    struct Counting<'a> {
        inner: crate::feed::MemorySource<'a>,
        lookups: std::sync::atomic::AtomicUsize,
    }

    impl crate::feed::InsightsSource for Counting<'_> {
        fn len(&self) -> usize {
            self.inner.len()
        }
        fn commit(&self, r: crate::feed::Rank) -> Option<crate::feed::CommitRow> {
            self.inner.commit(r)
        }
        fn commits_in(
            &self,
            r: crate::feed::CommitRange,
        ) -> crate::feed::Stream<'_, crate::feed::CommitRow> {
            self.inner.commits_in(r)
        }
        fn rows_for(
            &self,
            c: Vec<crate::feed::Rank>,
        ) -> crate::feed::Stream<'_, (crate::feed::Rank, Vec<crate::feed::FileRow>)> {
            self.inner.rows_for(c)
        }
        fn commit_id(&self, r: crate::feed::Rank) -> Option<std::sync::Arc<str>> {
            self.inner.commit_id(r)
        }
        fn parent_ids(&self, r: crate::feed::Rank) -> Vec<std::sync::Arc<str>> {
            self.inner.parent_ids(r)
        }
        fn find_commit(&self, id: &str) -> Option<crate::feed::Rank> {
            self.inner.find_commit(id)
        }
        fn agg_author_day(
            &self,
            d: std::ops::Range<i64>,
        ) -> crate::feed::Stream<'_, crate::feed::AuthorDayRow> {
            self.inner.agg_author_day(d)
        }
        fn agg_path_author(&self, p: &str) -> crate::feed::Stream<'_, crate::feed::PathAuthorRow> {
            self.inner.agg_path_author(p)
        }
        fn postings(&self, k: crate::feed::Posting) -> crate::feed::Stream<'_, crate::feed::Hit> {
            self.lookups
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.inner.postings(k)
        }
        fn rename_edges(&self) -> crate::feed::Stream<'_, crate::feed::RenameEdge> {
            self.inner.rename_edges()
        }
        fn path(&self, id: crate::feed::PathId) -> Option<std::sync::Arc<str>> {
            self.inner.path(id)
        }
        fn find_path(&self, p: &str) -> Option<crate::feed::PathId> {
            self.inner.find_path(p)
        }
        fn paths(&self) -> usize {
            self.inner.paths()
        }
        fn ident(
            &self,
            id: crate::feed::IdentId,
        ) -> Option<(std::sync::Arc<str>, std::sync::Arc<str>)> {
            self.inner.ident(id)
        }
        fn idents(&self) -> usize {
            self.inner.idents()
        }
        fn label(&self, id: crate::feed::LabelId) -> Option<std::sync::Arc<str>> {
            self.inner.label(id)
        }
        fn find_label(&self, n: &str) -> Option<crate::feed::LabelId> {
            self.inner.find_label(n)
        }
    }

    /// Hotspots, ownership and code age over many head files read the postings a fixed number
    /// of times, not once per file: a lookup per file is tens of thousands of block reads on a
    /// large tree. Red on the per-file loop.
    #[test]
    fn file_pages_read_postings_a_fixed_number_of_times() {
        let n = 500;
        let recs: Vec<crate::CommitRecord> = (0..n)
            .map(|i| {
                rec(
                    &format!("c{i}"),
                    "Ann",
                    i64::from(i),
                    vec![file(&format!("f{i}.rs"), 1, 0, 1)],
                )
            })
            .collect();
        let hf: Vec<HeadFile> = (0..n)
            .map(|i| HeadFile {
                path: format!("f{i}.rs"),
                lines: 1,
            })
            .collect();
        let src = Counting {
            inner: crate::feed::MemorySource::new(&recs),
            lookups: std::sync::atomic::AtomicUsize::new(0),
        };
        let ins = Insights::from_source(&src, Options::default(), None);
        let read = || src.lookups.swap(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(ins.hotspots(&hf, Period::All).files.len(), n as usize);
        let hot = read();
        drop(ins.ownership(&hf, Period::All, 1));
        let own = read();
        drop(ins.code_age(&hf));
        let age = read();
        assert!(hot + own + age <= 3, "postings lookups: {hot} {own} {age}");
    }

    /// Over all history, ownership read from the path x author aggregate equals ownership
    /// from the file row touches, with renames, bots, whitespace and a file nobody renamed.
    #[test]
    fn ownership_from_the_aggregate_equals_the_touches() {
        let mut recs = history();
        recs.push(rec(
            "e",
            "dependabot[bot]",
            403,
            vec![file("keep.rs", 9, 2, 5)],
        ));
        // Whitespace-only lines: raw and code counts differ.
        recs.push(rec(
            "f",
            "Bob",
            404,
            vec![FileStat {
                code: Some(crate::CodeStat {
                    additions: 1,
                    deletions: 0,
                    rewritten: 0,
                }),
                ..file("keep.rs", 6, 4, 5)
            }],
        ));
        let hf = head(&[("new/x.rs", 14), ("keep.rs", 5), ("gone.rs", 3)]);
        for (bots, ws) in [(false, false), (true, false), (false, true), (true, true)] {
            let opts = Options {
                include_bots: bots,
                count_whitespace: ws,
                ..Options::default()
            };
            let ins = Insights::new(&recs, opts, None);
            let shown: Vec<&HeadFile> = hf.iter().collect();
            let fast = ins.owner_lines(&shown, i64::MIN, i64::MAX);
            let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
            let slow: Vec<std::collections::BTreeMap<usize, u64>> = ins
                .touches_in(&names, i64::MIN, i64::MAX)
                .into_iter()
                .map(|t| {
                    let mut m = std::collections::BTreeMap::new();
                    for (t, c) in t {
                        *m.entry(ins.person_of(c.ident)).or_default() += u64::from(t.lines);
                    }
                    m
                })
                .collect();
            assert_eq!(fast, slow, "bots {bots} ws {ws}");
        }
    }

    /// A name reused after a rename: `a.rs` moved to `b.rs`, then a new `a.rs` appeared.
    /// The path x author aggregate holds the old file's rows under `a.rs`, so the hybrid must
    /// not read it for `a.rs` (nor `b.rs`): both answer as the row touches do. Red when the
    /// split only checks whether the head name itself was renamed away.
    #[test]
    fn a_reused_name_takes_the_touches_not_the_aggregate() {
        let recs = vec![
            rec("a", "Ann", 0, vec![file("a.rs", 10, 0, 10)]),
            rec("b", "Ann", 10, vec![renamed("a.rs", "b.rs")]),
            rec("c", "Bob", 20, vec![file("a.rs", 3, 0, 3)]),
        ];
        let hf = head(&[("a.rs", 3), ("b.rs", 10)]);
        let ins = Insights::new(&recs, Options::default(), None);
        let shown: Vec<&HeadFile> = hf.iter().collect();
        let fast = ins.owner_lines(&shown, i64::MIN, i64::MAX);
        let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
        let slow: Vec<std::collections::BTreeMap<usize, u64>> = ins
            .touches_in(&names, i64::MIN, i64::MAX)
            .into_iter()
            .map(|t| {
                let mut m = std::collections::BTreeMap::new();
                for (t, c) in t {
                    *m.entry(ins.person_of(c.ident)).or_default() += u64::from(t.lines);
                }
                m
            })
            .collect();
        assert_eq!(fast, slow);
        let ann = ins.person_of(crate::feed::IdentId(0));
        assert!(
            !fast[0].contains_key(&ann),
            "the new a.rs has only Bob's lines"
        );
    }
}
