//! Lines per language on the first-parent chain without holding the chain.
//!
//! A [`Skeleton`] is built once per head by one streamed walk. It keeps the positions the
//! unranged page reads and a capped set of marks (lines and running add and delete sums after
//! a commit), so its size is flat in history length. A ranged view resolves the few other
//! positions it reads by walking from the nearest newer mark, at most one mark gap plus the
//! commit list.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use gitterrace_index::PathFilter;
use gitterrace_index::feed::{FileRow, InsightsSource, PathId, Rank};

use crate::languages::{COMMIT_LIST, TOP, View, shown};

/// Commits per row batch: bounded, and large enough that a batch read stays sequential.
const CHUNK: usize = 4096;
/// Most marks a skeleton holds, whatever the history length.
const MARKS: usize = 2048;
/// Fewest commits between two marks, so a short history does not hold a mark per commit.
const MIN_GAP: usize = 64;

/// One chain position as the page reads it.
#[derive(Clone)]
pub(crate) struct Point {
    id: String,
    time: i64,
    offset: i32,
    /// Lines per shown language after this commit, `Other` last when folded.
    lines: Vec<i64>,
    /// Lines added and removed from the oldest commit through this one.
    added: u64,
    removed: u64,
}

/// Tally state at one commit: its rank, net lines per slot and the added and removed sums.
type Snapshot = (Rank, Vec<i64>, u64, u64);

/// The state after one commit, from which older positions are walked.
struct Mark {
    rank: Rank,
    lines: Vec<i64>,
    added: u64,
    removed: u64,
}

/// What every view of the languages page shares, sized by the page, not the history.
pub(crate) struct Skeleton {
    n: usize,
    /// Top languages, largest first, then `Other` when folded.
    langs: Vec<String>,
    /// Commits between marks; the mark of position `p` sits at the end of gap `p / gap`.
    gap: usize,
    marks: Vec<Mark>,
    fixed: HashMap<usize, Point>,
}

/// One view's positions: the skeleton plus the positions only this view reads.
pub(crate) struct Chain {
    sk: Arc<Skeleton>,
    extra: HashMap<usize, Point>,
    /// Lines added and removed over the view's range (the whole chain without one).
    pub(super) added: u64,
    pub(super) removed: u64,
}

/// Net lines per language slot over the commits applied so far.
struct Tally<'a> {
    src: &'a dyn InsightsSource,
    filter: &'a PathFilter,
    names: Vec<String>,
    slot: HashMap<String, usize>,
    /// Language slot per path id, `None` when the path filter leaves it out.
    lang_of: HashMap<PathId, Option<usize>>,
    net: Vec<i64>,
    added: u64,
    removed: u64,
}

impl<'a> Tally<'a> {
    fn new(src: &'a dyn InsightsSource, filter: &'a PathFilter) -> Self {
        Self {
            src,
            filter,
            names: Vec::new(),
            slot: HashMap::new(),
            lang_of: HashMap::new(),
            net: Vec::new(),
            added: 0,
            removed: 0,
        }
    }

    fn apply(&mut self, files: &[FileRow]) {
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
        let l = gitterrace_index::language(&path);
        if let Some(s) = self.slot.get(&l) {
            return Some(*s);
        }
        self.names.push(l.clone());
        self.net.push(0);
        self.slot.insert(l, self.names.len() - 1);
        Some(self.names.len() - 1)
    }

    /// The tally's current state, saved to be subtracted from the final totals.
    fn snapshot(&self, rank: Rank) -> Snapshot {
        (rank, self.net.clone(), self.added, self.removed)
    }
}

/// The first-parent chain from `rank`, newest first.
fn walk(src: &dyn InsightsSource, rank: Option<Rank>) -> impl Iterator<Item = Rank> + '_ {
    std::iter::successors(rank, move |r| {
        src.parent_ids(*r).first().and_then(|p| src.find_commit(p))
    })
}

/// Streams the chain from `rank` in bounded ascending batches, yielding newest first.
fn rows_newest_first(
    src: &dyn InsightsSource,
    rank: Option<Rank>,
    take: usize,
    mut each: impl FnMut(Rank, &[FileRow]),
) {
    let mut ranks = walk(src, rank).take(take);
    loop {
        let mut chunk: Vec<Rank> = ranks.by_ref().take(CHUNK).collect();
        if chunk.is_empty() {
            break;
        }
        chunk.reverse();
        let mut batch: Vec<(Rank, Vec<FileRow>)> = src.rows_for(chunk).collect();
        while let Some((r, files)) = batch.pop() {
            each(r, &files);
        }
    }
}

impl Skeleton {
    /// Counts the chain, then streams its rows once, keeping the unranged view's positions
    /// and one mark per gap.
    pub(crate) fn build(src: &dyn InsightsSource, filter: &PathFilter) -> Self {
        let newest = src
            .len()
            .checked_sub(1)
            .map(|i| Rank(u32::try_from(i).unwrap_or(u32::MAX)));
        let n = walk(src, newest).count();
        let gap = n.div_ceil(MARKS).max(MIN_GAP);
        let want = wanted(n, None);
        let mut tally = Tally::new(src, filter);
        // Tally state over the commits newer than each kept position.
        let mut fixed_raw: Vec<(usize, Snapshot)> = Vec::new();
        let mut marks_raw: Vec<Snapshot> = Vec::new();
        let mut pos = n;
        rows_newest_first(src, newest, n, |r, files| {
            pos -= 1;
            if want.contains(&pos) {
                fixed_raw.push((pos, tally.snapshot(r)));
            }
            if pos % gap == gap - 1 || pos == n - 1 {
                marks_raw.push(tally.snapshot(r));
            }
            tally.apply(files);
        });
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
        marks_raw.reverse();
        let marks = marks_raw
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
        let fixed = fixed_raw
            .iter()
            .map(|(p, m)| {
                let (lines, added, removed) = after(m);
                (*p, point(src, m.0, lines, added, removed))
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

    pub(crate) const fn len(&self) -> usize {
        self.n
    }

    /// Bytes held, for the bound the tests keep flat in history length.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        let k = self.langs.len() * 8;
        self.marks.len() * (std::mem::size_of::<Mark>() + k)
            + self.fixed.len() * (std::mem::size_of::<Point>() + k + 40 + 8)
    }

    /// Resolves the positions `view` reads that the skeleton does not keep.
    pub(super) fn view(
        self: &Arc<Self>,
        src: &dyn InsightsSource,
        filter: &PathFilter,
        view: View,
    ) -> Chain {
        let mut need: BTreeSet<usize> = wanted(self.n, view.range);
        need.retain(|p| !self.fixed.contains_key(p));
        let mut extra: HashMap<usize, Point> = HashMap::new();
        let mut lang_of: HashMap<PathId, Option<usize>> = HashMap::new();
        let other =
            (self.langs.last().map(String::as_str) == Some("Other")).then(|| self.langs.len() - 1);
        let named = other.unwrap_or(self.langs.len());
        let mut slot_of = |id: PathId| -> Option<usize> {
            *lang_of.entry(id).or_insert_with(|| {
                let path = src.path(id).unwrap_or_default();
                if filter.skips(&path) {
                    return None;
                }
                let l = gitterrace_index::language(&path);
                self.langs[..named].iter().position(|x| *x == l).or(other)
            })
        };
        while let Some(&top) = need.iter().next_back() {
            let g = top / self.gap;
            let Some(mark) = self.marks.get(g) else {
                break;
            };
            let start = (g * self.gap + self.gap - 1).min(self.n - 1);
            let low = need
                .range(g * self.gap..=top)
                .next()
                .copied()
                .unwrap_or(top);
            let low = low.min(top);
            let (mut lines, mut added, mut removed) =
                (mark.lines.clone(), mark.added, mark.removed);
            let mut pos = start + 1;
            rows_newest_first(src, Some(mark.rank), start - low + 1, |r, files| {
                pos -= 1;
                if need.contains(&pos) {
                    extra.insert(pos, point(src, r, lines.clone(), added, removed));
                }
                for f in files {
                    if let Some(s) = slot_of(f.path) {
                        lines[s] -= i64::from(f.add) - i64::from(f.del);
                        added -= u64::from(f.add);
                        removed -= u64::from(f.del);
                    }
                }
            });
            need.retain(|p| *p < low);
        }
        let mut ch = Chain {
            sk: Arc::clone(self),
            extra,
            added: 0,
            removed: 0,
        };
        if self.n > 0 {
            let (from, to) = view.range.unwrap_or((0, self.n - 1));
            let before = |c: &Chain| if from == 0 { (0, 0) } else { c.sums(from - 1) };
            let (a0, d0) = before(&ch);
            let (a1, d1) = ch.sums(to);
            ch.added = a1 - a0;
            ch.removed = d1 - d0;
        }
        ch
    }
}

impl Chain {
    fn at(&self, i: usize) -> Option<&Point> {
        self.sk.fixed.get(&i).or_else(|| self.extra.get(&i))
    }

    fn sums(&self, i: usize) -> (u64, u64) {
        self.at(i).map_or((0, 0), |p| (p.added, p.removed))
    }

    pub(super) fn len(&self) -> usize {
        self.sk.n
    }

    pub(super) fn langs(&self) -> &[String] {
        &self.sk.langs
    }

    pub(super) fn id(&self, i: usize) -> &str {
        self.at(i).map_or("", |p| &p.id)
    }

    pub(super) fn time(&self, i: usize) -> i64 {
        self.at(i).map_or(0, |p| p.time)
    }

    pub(super) fn offset(&self, i: usize) -> i32 {
        self.at(i).map_or(0, |p| p.offset)
    }

    /// Lines per language after commit `i`.
    pub(super) fn lines(&self, i: usize) -> Vec<i64> {
        self.at(i)
            .map_or_else(|| vec![0; self.sk.langs.len()], |p| p.lines.clone())
    }

    /// Lines per language before commit `i` (all zero before the first).
    pub(super) fn before(&self, i: usize) -> Vec<i64> {
        if i == 0 {
            vec![0; self.sk.langs.len()]
        } else {
            self.lines(i - 1)
        }
    }
}

/// Extensions that name a file format, never a language: signatures, archives, binaries and
/// build outputs. Their lines count under `Other`, so the legend lists only languages.
const NOT_LANGUAGES: &[&str] = &[
    ".asc",
    ".sig",
    ".gpg",
    ".pgp",
    ".minisig",
    ".sha256",
    ".sha512",
    ".md5",
    ".prebuilt",
    ".bin",
    ".exe",
    ".dll",
    ".so",
    ".dylib",
    ".a",
    ".o",
    ".obj",
    ".lib",
    ".class",
    ".jar",
    ".wasm",
    ".pyc",
    ".gz",
    ".tgz",
    ".bz2",
    ".xz",
    ".zst",
    ".zip",
    ".tar",
    ".7z",
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".ico",
    ".icns",
    ".webp",
    ".bmp",
    ".pdf",
    ".woff",
    ".woff2",
    ".ttf",
    ".otf",
    ".eot",
    ".mp3",
    ".mp4",
    ".mov",
    ".wav",
    ".lock",
    ".map",
    ".snap",
];

/// Whether the slot `name` is a language the legend may name.
pub(crate) fn is_language(name: &str) -> bool {
    !NOT_LANGUAGES.contains(&name)
}

/// Which language slots the page shows, by their lines at the newest commit.
struct Fold {
    langs: Vec<String>,
    keep: Vec<usize>,
    folded: bool,
}

impl Fold {
    fn new(names: &[String], last: &[i64]) -> Self {
        let mut order: Vec<usize> = (0..names.len())
            .filter(|s| last[*s] > 0 && is_language(&names[*s]))
            .collect();
        order.sort_by(|a, b| {
            last[*b]
                .cmp(&last[*a])
                .then_with(|| names[*a].cmp(&names[*b]))
        });
        // Leaving out a format folds it, so "Other" must show even under the cut.
        let formats = (0..names.len()).any(|s| last[s] != 0 && !is_language(&names[s]));
        let keep: Vec<usize> = if order.len() <= TOP + usize::from(!formats) {
            order
        } else {
            order[..TOP].to_vec()
        };
        let folded = names.len() > keep.len();
        let mut langs: Vec<String> = keep.iter().map(|s| names[*s].clone()).collect();
        if folded {
            langs.push("Other".to_owned());
        }
        Self {
            langs,
            keep,
            folded,
        }
    }

    fn apply(&self, row: &[i64]) -> Vec<i64> {
        let mut v: Vec<i64> = self.keep.iter().map(|s| row[*s]).collect();
        if self.folded {
            let kept: i64 = v.iter().sum();
            let all: i64 = row.iter().sum();
            v.push(all - kept);
        }
        v
    }
}

fn point(src: &dyn InsightsSource, r: Rank, lines: Vec<i64>, added: u64, removed: u64) -> Point {
    let c = src.commit(r);
    Point {
        id: src.commit_id(r).map(|s| s.to_string()).unwrap_or_default(),
        time: c.map_or(0, |c| c.time),
        offset: c.map_or(0, |c| c.offset),
        lines,
        added,
        removed,
    }
}

/// Every position the page reads for a range: the shown commits, the chain ends, the
/// commit before the range and before the last ten, and the trend samples.
fn wanted(n: usize, range: Option<(usize, usize)>) -> BTreeSet<usize> {
    let view = View {
        time: false,
        share: false,
        range,
    };
    let mut w: BTreeSet<usize> = shown(n, view).into_iter().collect();
    if n == 0 {
        return w;
    }
    w.extend([0, n - 1]);
    w.extend(n.saturating_sub(COMMIT_LIST)..n);
    if let Some((from, _)) = range
        && from > 0
    {
        w.insert(from - 1);
    }
    let back = n.saturating_sub(10);
    if back > 0 {
        w.insert(back - 1);
    }
    w.extend((0..n).step_by((n / 40).max(1)));
    w
}

#[cfg(test)]
mod tests;
