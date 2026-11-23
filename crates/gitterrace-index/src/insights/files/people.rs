//! Knowledge risk: who holds each directory of the head tree, whether they are still around,
//! and the files nobody active holds.
//!
//! Attribution is the one `ownership` uses, by lines changed: [`Insights::owner_lines`] per
//! head file, weighted by the file's size now. A person is idle when their newest commit is
//! more than `STALE_SECS` older than the history's newest time, the rule of `stale_lines`.
//! The scan reads the files in chunks and keeps only a per-directory table (people, not files)
//! and two bounded heaps, so memory does not grow with the tree.

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};

use crate::insights::files::{STALE_SECS, reports::Acc};
use crate::insights::{Counted, HeadFile, Insights, Period, bus_factor};

/// Rows an orphan or file list holds when the caller names no limit.
/// Not configurable: a page of the largest rows.
pub const RISK_LIMIT: usize = 50;
/// Most rows an orphan or file list holds, whatever the caller asks, and most directories
/// listed. Not configurable: it bounds the heaps, so memory does not scale with files.
pub const RISK_MAX_LIMIT: usize = 500;
/// Head files read per visit of the touches.
/// Not configurable: it bounds the per-file maps held at once, the answer is the same.
const RISK_CHUNK: usize = 4096;
/// A person with at least this share of a file's changes counts as one of its main authors.
/// Not configurable: it defines an orphan, so a setting would change the numbers.
const MAIN_SHARE: f64 = 0.1;

/// What to count: see the module docs.
#[derive(Debug, Clone, Copy, Default)]
pub struct RiskQuery<'a> {
    /// The directory whose children are listed. `None` is the root.
    pub path: Option<&'a str>,
    /// The person (a name as `ownership` lists it) to report, `None` for nobody.
    pub author: Option<&'a str>,
    /// Most orphans and most top files listed, at most [`RISK_MAX_LIMIT`].
    pub limit: usize,
}

/// One child directory of the drilled path (or the files directly in it, under the path itself).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RiskDir {
    pub path: String,
    pub lines: u64,
    pub files: usize,
    /// People who together hold at least half of the knowledge.
    pub bus_factor: usize,
    pub top_owner: String,
    pub top_share: f64,
    /// The top owner committed within `STALE_SECS` of the newest commit.
    pub owner_active: bool,
    /// One person holds half of it and has not committed in the last 12 months.
    pub at_risk: bool,
    /// Author time of the top owner's newest commit.
    pub last_active: i64,
}

/// A file whose main authors are all idle.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Orphan {
    pub path: String,
    pub lines: u64,
    /// Author time of the newest counted change to the file.
    pub last_change: i64,
}

/// A file a person owns, most lines first.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PersonFile {
    pub path: String,
    pub lines: u64,
    pub share: f64,
}

/// What one person holds: the files where they are the top owner.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RiskPerson {
    pub name: String,
    pub files: usize,
    pub lines: u64,
    /// `lines` over the lines of every tracked head file.
    pub share_of_repo: f64,
    pub last_active: i64,
    pub top_files: Vec<PersonFile>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct KnowledgeRisk {
    pub dirs: Vec<RiskDir>,
    pub orphans: Vec<Orphan>,
    /// Orphans under the path, all of them, not only those listed.
    pub orphan_files: usize,
    pub orphan_lines: u64,
    pub person: Option<RiskPerson>,
    pub at_risk_lines: u64,
    pub at_risk_dirs: usize,
}

/// A candidate row of a bounded heap: larger lines first, then the smaller path.
struct Ranked {
    lines: u64,
    path: String,
    share: f64,
}

impl PartialEq for Ranked {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Ranked {}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Ranked {
    fn cmp(&self, o: &Self) -> Ordering {
        self.lines.cmp(&o.lines).then_with(|| o.path.cmp(&self.path))
    }
}

/// The best `k` rows offered, held in a min-heap so memory is `k` rows whatever is offered.
struct Top {
    k: usize,
    heap: BinaryHeap<Reverse<Ranked>>,
}

impl Top {
    const fn new(k: usize) -> Self {
        Self { k, heap: BinaryHeap::new() }
    }

    /// Whether a row of `lines` and `path` would be kept: asked before the row is built.
    fn wants(&self, lines: u64, path: &str) -> bool {
        self.k > 0
            && (self.heap.len() < self.k
                || self.heap.peek().is_some_and(|Reverse(w)| {
                    (lines, Reverse(path)) > (w.lines, Reverse(&*w.path))
                }))
    }

    fn push(&mut self, r: Ranked) {
        self.heap.push(Reverse(r));
        if self.heap.len() > self.k {
            self.heap.pop();
        }
    }

    fn sorted(self) -> Vec<Ranked> {
        let mut v: Vec<Ranked> = self.heap.into_iter().map(|Reverse(r)| r).collect();
        v.sort_by(|a, b| b.cmp(a));
        v
    }
}

/// What the scan of the head files gathers: people per directory, two bounded heaps and counts.
struct Scan {
    dirs: BTreeMap<String, Acc>,
    orphans: Top,
    orphan_files: usize,
    orphan_lines: u64,
    mine: usize,
    mine_lines: u64,
    tracked: u64,
    owned: Top,
}

impl Scan {
    const fn new(limit: usize) -> Self {
        Self {
            dirs: BTreeMap::new(),
            orphans: Top::new(limit),
            orphan_files: 0,
            orphan_lines: 0,
            mine: 0,
            mine_lines: 0,
            tracked: 0,
            owned: Top::new(limit),
        }
    }
}

/// What a scan reads about the request: who, where, and who is idle.
struct Ask<'a> {
    who: Option<usize>,
    prefix: Option<&'a str>,
    idle_before: i64,
}

impl Insights<'_> {
    /// Whether a person of this name made a commit in the history.
    pub fn knows(&self, name: &str) -> bool {
        self.d.people.list.iter().any(|p| p.name == name)
    }

    /// The knowledge risk of the head tree under `q.path` for the commits `c` counts, `None`
    /// when `q.author` names nobody.
    pub fn knowledge_risk_in(
        &self,
        files: &[HeadFile],
        c: &Counted<'_>,
        q: &RiskQuery<'_>,
    ) -> Option<KnowledgeRisk> {
        let who = match q.author {
            Some(name) => Some(self.d.people.list.iter().position(|p| p.name == name)?),
            None => None,
        };
        let base = q.path.map(|p| p.trim_matches('/')).filter(|p| !p.is_empty());
        let prefix = base.map(|p| format!("{p}/"));
        let ask = Ask {
            who,
            prefix: prefix.as_deref(),
            idle_before: self.now.saturating_sub(STALE_SECS),
        };
        let m = c.scope().matcher();
        // The person block reads the whole tree; the directories and orphans only the subtree.
        let scan: Vec<&HeadFile> = files
            .iter()
            .filter(|f| f.lines != 0 && m.row(&f.path) && !self.skips_head(&f.path))
            .filter(|f| who.is_some() || ask.prefix.is_none_or(|x| f.path.starts_with(x)))
            .collect();
        let s = c.scope();
        let whole = s.period() == Period::All && s.range().is_none() && s.matcher().is_empty();
        let chunk = if self.source().has_tip() { RISK_CHUNK } else { usize::MAX };
        let mut got = Scan::new(q.limit.min(RISK_MAX_LIMIT));
        for part in scan.chunks(chunk.min(scan.len().max(1))) {
            for (file, held) in part.iter().zip(self.owner_lines(part, c, whole)) {
                self.take(&mut got, &ask, file, &held);
            }
        }
        Some(self.finish(got, who, c))
    }

    /// Adds one head file and the lines each person changed in it.
    fn take(&self, got: &mut Scan, ask: &Ask<'_>, file: &HeadFile, held: &BTreeMap<usize, u64>) {
        let changed: u64 = held.values().sum();
        if changed == 0 {
            return;
        }
        let top = held
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| self.person_order(*b.0, *a.0)))
            .map_or(0, |(p, _)| *p);
        let lines = u64::from(file.lines);
        got.tracked += lines;
        let mine = ask.who == Some(top);
        if mine {
            got.mine += 1;
            got.mine_lines += lines;
            if got.owned.wants(lines, &file.path) {
                let share = held[&top] as f64 / changed as f64;
                got.owned.push(Ranked { lines, path: file.path.clone(), share });
            }
        }
        let cut = ask.prefix.map_or(0, str::len);
        if !file.path.starts_with(ask.prefix.unwrap_or_default()) {
            return;
        }
        let key = match file.path[cut..].split_once('/') {
            Some((dir, _)) => &file.path[..cut + dir.len()],
            None => &file.path[..cut.saturating_sub(1)],
        };
        let acc = got.dirs.entry(key.to_owned()).or_default();
        for (&p, &n) in held {
            *acc.w.entry(p).or_default() += lines as f64 * n as f64 / changed as f64;
        }
        acc.lines += lines;
        acc.files += 1;
        let quiet = |p: usize| self.d.last_seen.get(p).is_some_and(|&t| t < ask.idle_before);
        let main = |p: usize, n: u64| p == top || n as f64 / changed as f64 >= MAIN_SHARE;
        let orphan = held.iter().filter(|&(&p, &n)| main(p, n)).all(|(&p, _)| quiet(p));
        if orphan && (ask.who.is_none() || mine) {
            got.orphan_files += 1;
            got.orphan_lines += lines;
            if got.orphans.wants(lines, &file.path) {
                got.orphans.push(Ranked { lines, path: file.path.clone(), share: 0.0 });
            }
        }
    }

    /// The report of a finished scan.
    fn finish(&self, got: Scan, who: Option<usize>, c: &Counted<'_>) -> KnowledgeRisk {
        let mut rows: Vec<(RiskDir, usize)> =
            got.dirs.into_iter().map(|(path, a)| self.risk_dir(path, a)).collect();
        // The totals cover every directory under the path, before a person or the cap cuts.
        let (at_risk_lines, at_risk_dirs) = rows
            .iter()
            .filter(|(d, _)| d.at_risk)
            .fold((0, 0), |(l, n), (d, _)| (l + d.lines, n + 1));
        if let Some(w) = who {
            rows.retain(|(_, top)| *top == w);
        }
        let mut dirs: Vec<RiskDir> = rows.into_iter().map(|(d, _)| d).collect();
        dirs.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.path.cmp(&b.path)));
        dirs.truncate(RISK_MAX_LIMIT);
        let person = who.map(|w| RiskPerson {
            name: self.person_at(w).map(|p| p.name.clone()).unwrap_or_default(),
            files: got.mine,
            lines: got.mine_lines,
            share_of_repo: if got.tracked == 0 {
                0.0
            } else {
                got.mine_lines as f64 / got.tracked as f64
            },
            last_active: self.d.last_seen.get(w).copied().unwrap_or(i64::MIN),
            top_files: got
                .owned
                .sorted()
                .into_iter()
                .map(|r| PersonFile { path: r.path, lines: r.lines, share: r.share })
                .collect(),
        });
        KnowledgeRisk {
            dirs,
            orphans: self.dated(got.orphans.sorted(), c),
            orphan_files: got.orphan_files,
            orphan_lines: got.orphan_lines,
            person,
            at_risk_lines,
            at_risk_dirs,
        }
    }

    /// One directory's row and its top owner's person index.
    fn risk_dir(&self, path: String, a: Acc) -> (RiskDir, usize) {
        let mut owners: Vec<(usize, f64)> = a.w.into_iter().collect();
        owners.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| self.person_order(x.0, y.0)));
        let total: f64 = owners.iter().map(|o| o.1).sum::<f64>().max(f64::MIN_POSITIVE);
        let bus = bus_factor(owners.iter().map(|o| o.1));
        let (top, w) = owners.first().copied().unwrap_or((0, 0.0));
        let last = self.d.last_seen.get(top).copied().unwrap_or(i64::MIN);
        let active = last >= self.now.saturating_sub(STALE_SECS);
        let row = RiskDir {
            path,
            lines: a.lines,
            files: a.files,
            bus_factor: bus,
            top_owner: self.person_at(top).map(|p| p.name.clone()).unwrap_or_default(),
            top_share: w / total,
            owner_active: active,
            at_risk: bus == 1 && !active,
            last_active: last,
        };
        (row, top)
    }

    /// The orphans with the time of their newest counted change, read for the kept rows only.
    fn dated(&self, rows: Vec<Ranked>, c: &Counted<'_>) -> Vec<Orphan> {
        let names: Vec<&str> = rows.iter().map(|r| &*r.path).collect();
        let mut last = vec![i64::MIN; rows.len()];
        // Touches come newest first per path: the first is the one wanted.
        self.visit_touches_in(&names, c, |i, _, _, row| {
            last[i] = last[i].max(row.time);
            false
        });
        rows.into_iter()
            .zip(last)
            .map(|(r, last_change)| Orphan { path: r.path, lines: r.lines, last_change })
            .collect()
    }
}

#[cfg(test)]
mod tests;
