//! Explorable coupling: the pairs of head files that changed in the same commits, for one
//! focus file or for the whole tree, and the same counts rolled up to top-level directories.
//!
//! A commit counts when it touches 2 to `COUPLING_MAX_FILES` head files: a bigger one is a
//! sweep (formatting, renames, vendoring) and is only counted in `skipped_commits`, so the
//! threshold is visible. Merges carry no files and never pair. A file changes once per
//! commit, however many rows name it. With a focus file only the pairs that name it are
//! counted, one dense row for it, so the cost is its commits times their size, never the
//! square of the files. Without one every file gets its row in turn and the candidates are
//! kept in a list that is cut back to `limit` whenever it reaches `COUPLING_PAIR_BUDGET`,
//! which never changes the answer: a pair cut is below `limit` others under the same order.

use std::cmp::Ordering;

use rustc_hash::FxHashMap as FastMap;

use crate::insights::files::COUPLING_MAX_FILES;
use crate::insights::files::reports::under_path;
use crate::insights::{Counted, HeadFile, Insights};

/// Candidate pairs held before the list is cut back to the limit asked for.
/// Not configurable: it bounds memory, and the answer is the same for any size.
pub(super) const COUPLING_PAIR_BUDGET: usize = 1 << 20;
/// Directory pairs the matrix keeps, most shared commits first.
/// Not configurable: the matrix is a summary, and a longer list is the pairs themselves.
pub(super) const COUPLING_MATRIX_PAIRS: usize = 64;

/// What to count: see the module docs.
#[derive(Debug, Clone, Copy)]
pub struct CouplingQuery<'a> {
    /// The focus file (a head tree name). `None` lists the strongest pairs of the tree.
    pub path: Option<&'a str>,
    /// Shared commits a pair needs, at least 1.
    pub min_support: usize,
    /// Keep pairs whose files sit in different directories too, not only siblings.
    pub cross_dir: bool,
    /// Most pairs listed.
    pub limit: usize,
}

pub use gitterrace_types::api::{
    InsightCoupling as CouplingReport, InsightCouplingPair as CouplingPair,
    InsightDirPair as DirPair,
};

/// (file a, file b, together, newest shared commit time), a before b.
type Cand = (u32, u32, usize, i64);

/// The counted commits as flat tables: per file its commits, per commit its files.
struct Groups {
    changes: Vec<usize>,
    /// Start of each file's touches in `touches`.
    start: Vec<usize>,
    /// (rank, file), files ascending, a commit once per file.
    touches: Vec<(u32, u32)>,
    /// Per rank, the start of its files in `members`; commits that do not count are empty.
    begin: Vec<u32>,
    /// Files of each counted commit, ascending.
    members: Vec<u32>,
    /// Author time per rank.
    time: Vec<i64>,
    skipped: usize,
}

impl Groups {
    fn commit(&self, rank: u32) -> &[u32] {
        let (a, b) = (self.begin[rank as usize], self.begin[rank as usize + 1]);
        &self.members[a as usize..b as usize]
    }

    fn of(&self, file: u32) -> &[(u32, u32)] {
        &self.touches[self.start[file as usize]..self.start[file as usize + 1]]
    }
}

/// One dense row of co-change counts, reused for every file.
struct Walk<'a> {
    head: &'a [&'a str],
    g: &'a Groups,
    min: usize,
    cross: bool,
    row: Vec<usize>,
    last: Vec<i64>,
    hit: Vec<u32>,
}

impl Walk<'_> {
    /// Pushes the pairs of file `a`: with every file when `all`, else with the later ones.
    fn file(&mut self, a: u32, all: bool, out: &mut Vec<Cand>) {
        if self.g.changes[a as usize] < self.min {
            return;
        }
        for &(rank, _) in self.g.of(a) {
            for &b in self.g.commit(rank) {
                if b == a || (!all && b < a) {
                    continue;
                }
                if self.row[b as usize] == 0 {
                    self.hit.push(b);
                }
                self.row[b as usize] += 1;
                let t = self.g.time[rank as usize];
                self.last[b as usize] = self.last[b as usize].max(t);
            }
        }
        for b in std::mem::take(&mut self.hit) {
            let n = std::mem::take(&mut self.row[b as usize]);
            let last = std::mem::replace(&mut self.last[b as usize], i64::MIN);
            let same = parent(self.head[a as usize]) == parent(self.head[b as usize]);
            if n >= self.min && self.g.changes[b as usize] >= self.min && (self.cross || same) {
                out.push((a.min(b), a.max(b), n, last));
            }
        }
    }
}

/// The directory a file sits in, `""` at the root.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The first path segment when the file sits in a directory, else `""`.
fn top(path: &str) -> &str {
    path.split_once('/').map_or("", |(d, _)| d)
}

/// Keeps the best `limit` of `buf` under `order`.
fn cut(buf: &mut Vec<Cand>, limit: usize, order: &impl Fn(&Cand, &Cand) -> Ordering) {
    if buf.len() > limit {
        if limit > 0 {
            buf.select_nth_unstable_by(limit - 1, order);
        }
        buf.truncate(limit);
    }
}

impl Insights<'_> {
    /// The coupling of `files` over the commits `c` counts: see the module docs and
    /// [`CouplingQuery`].
    pub fn coupling_report_in(
        &self,
        files: &[HeadFile],
        set: &Counted<'_>,
        q: &CouplingQuery<'_>,
    ) -> CouplingReport {
        let min = q.min_support.max(1);
        let files = under_path(files, set);
        let mut head: Vec<&str> =
            files.iter().filter(|f| !self.skips_head(&f.path)).map(|f| &*f.path).collect();
        head.sort_unstable();
        head.dedup();
        let g = self.groups(&head, set);
        let mut walk = Walk {
            head: &head,
            g: &g,
            min,
            cross: q.cross_dir,
            row: vec![0; head.len()],
            last: vec![i64::MIN; head.len()],
            hit: Vec::new(),
        };
        let degree = |p: &Cand| {
            p.2 as f64 / g.changes[p.0 as usize].min(g.changes[p.1 as usize]).max(1) as f64
        };
        // Names sort as ids, so the id order is the name order.
        let order = |x: &Cand, y: &Cand| {
            y.2.cmp(&x.2)
                .then_with(|| degree(y).total_cmp(&degree(x)))
                .then_with(|| x.0.cmp(&y.0))
                .then_with(|| x.1.cmp(&y.1))
        };
        let mut found: Vec<Cand> = Vec::new();
        let mut matrix = Vec::new();
        if let Some(p) = q.path {
            if let Ok(a) = head.binary_search(&p) {
                walk.file(u32::try_from(a).unwrap_or(u32::MAX), true, &mut found);
            }
        } else {
            for a in 0..head.len() {
                walk.file(u32::try_from(a).unwrap_or(u32::MAX), false, &mut found);
                if found.len() >= COUPLING_PAIR_BUDGET {
                    cut(&mut found, q.limit, &order);
                }
            }
            matrix = dir_matrix(&head, &g, min);
        }
        cut(&mut found, q.limit, &order);
        found.sort_by(order);
        let pairs = found
            .iter()
            .map(|p| {
                let (left, right) = (head[p.0 as usize], head[p.1 as usize]);
                let (ac, bc) = (g.changes[p.0 as usize], g.changes[p.1 as usize]);
                CouplingPair {
                    a: left.to_owned(),
                    a_changes: ac,
                    b: right.to_owned(),
                    b_changes: bc,
                    confidence_ab: p.2 as f64 / ac as f64,
                    confidence_ba: p.2 as f64 / bc as f64,
                    cross_dir: parent(left) != parent(right),
                    degree: degree(p),
                    last_together: p.3,
                    together: p.2,
                }
            })
            .collect();
        CouplingReport {
            cross_dir: q.cross_dir,
            matrix,
            max_commit_files: COUPLING_MAX_FILES,
            min_support: min,
            pairs,
            path: q.path.map(str::to_owned),
            skipped_commits: g.skipped,
        }
    }

    /// The touches of `head` by the commits `c` counts, grouped both ways.
    fn groups(&self, head: &[&str], c: &Counted<'_>) -> Groups {
        let mut touches: Vec<(u32, u32)> = Vec::new();
        let mut changes = vec![0usize; head.len()];
        let mut time: Vec<i64> = Vec::new();
        let mut prev = (u32::MAX, u32::MAX);
        self.visit_touches_in(head, c, |i, _, _, row| {
            let key = (row.rank.0, u32::try_from(i).unwrap_or(u32::MAX));
            // A file named twice in one commit (a rename row and its new name) changes once.
            if key == prev {
                return true;
            }
            prev = key;
            touches.push(key);
            changes[i] += 1;
            let at = key.0 as usize;
            if time.len() <= at {
                time.resize(at + 1, i64::MIN);
            }
            time[at] = row.time;
            true
        });
        let mut start = Vec::with_capacity(head.len() + 1);
        let mut at = 0;
        for n in &changes {
            start.push(at);
            at += n;
        }
        start.push(at);
        let ranks = time.len();
        let mut size = vec![0u32; ranks];
        for t in &touches {
            size[t.0 as usize] += 1;
        }
        let counts = |n: u32| (2..=COUPLING_MAX_FILES as u32).contains(&n);
        let skipped = size.iter().filter(|n| **n as usize > COUPLING_MAX_FILES).count();
        let mut begin = vec![0u32; ranks + 1];
        for (r, n) in size.iter().enumerate() {
            begin[r + 1] = begin[r] + if counts(*n) { *n } else { 0 };
        }
        let mut fill = begin.clone();
        let mut members = vec![0u32; begin[ranks] as usize];
        for t in touches.iter().filter(|t| counts(size[t.0 as usize])) {
            let slot = &mut fill[t.0 as usize];
            members[*slot as usize] = t.1;
            *slot += 1;
        }
        Groups { changes, start, touches, begin, members, time, skipped }
    }
}

/// Commits touching both of two top-level directories, the strongest pairs first.
fn dir_matrix(head: &[&str], g: &Groups, min: usize) -> Vec<DirPair> {
    let mut counts: FastMap<(&str, &str), usize> = FastMap::default();
    let mut dirs: Vec<&str> = Vec::new();
    for rank in 0..g.begin.len() - 1 {
        let files = g.commit(u32::try_from(rank).unwrap_or(u32::MAX));
        if files.is_empty() {
            continue;
        }
        dirs.clear();
        dirs.extend(files.iter().map(|f| top(head[*f as usize])));
        dirs.sort_unstable();
        dirs.dedup();
        for (i, a) in dirs.iter().enumerate() {
            for b in &dirs[i + 1..] {
                *counts.entry((a, b)).or_default() += 1;
            }
        }
    }
    let mut rows: Vec<DirPair> = counts
        .into_iter()
        .filter(|(_, n)| *n >= min)
        .map(|((a, b), together)| DirPair { a: a.to_owned(), b: b.to_owned(), together })
        .collect();
    rows.sort_by(|x, y| {
        y.together.cmp(&x.together).then_with(|| x.a.cmp(&y.a)).then_with(|| x.b.cmp(&y.b))
    });
    rows.truncate(COUPLING_MATRIX_PAIRS);
    rows
}

#[cfg(test)]
mod tests;
