//! Lines per language on the first-parent chain without holding the chain.
//!
//! A [`Skeleton`] is built once per head by one streamed walk. It keeps the positions the
//! unranged page reads and a capped set of marks (lines and running add and delete sums after
//! a commit), so its size is flat in history length. A ranged view resolves the few other
//! positions it reads by walking from the nearest newer mark, at most one mark gap plus the
//! commit list.

mod build;

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use gitterrace_index::PathFilter;
use gitterrace_index::feed::{FileRow, InsightsSource, PathId, Rank};

use crate::languages::{COMMIT_LIST, Held, TOP, View, shown};

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

/// The first-parent chain from `rank`, newest first.
fn walk(src: &dyn InsightsSource, rank: Option<Rank>) -> impl Iterator<Item = Rank> + '_ {
    std::iter::successors(rank, move |r| first_parent(src, *r))
}

/// The rank of `r`'s first parent. Canonical order puts every parent below its child, so the
/// parent is found by reading the ranks below `r`, newest first. Those records sit in the
/// blocks just read, where an oid index lookup per step reads a random block (most of a
/// minute on rust). A chain walk reads each rank at most once this way, since the next step
/// starts below the parent. A parent outside the set is found by the lookup.
fn first_parent(src: &dyn InsightsSource, r: Rank) -> Option<Rank> {
    let p = src.parent_ids(r).into_iter().next()?;
    (0..r.0)
        .rev()
        .map(Rank)
        .find(|k| src.commit_id(*k).as_deref() == Some(&*p))
        .or_else(|| src.commit_of(&p))
}

/// Streams the rows of `ranks` (newest first) in bounded ascending batches, yielding newest
/// first.
fn rows_newest_first(
    src: &dyn InsightsSource,
    ranks: impl IntoIterator<Item = Rank>,
    mut each: impl FnMut(Rank, &[FileRow]),
) {
    let mut ranks = ranks.into_iter();
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
    pub(crate) const fn len(&self) -> usize {
        self.n
    }

    /// The oldest chain position whose local day is `first_day` or later, if any. One read
    /// of the stored chain, else one walk over commit rows; no file rows either way.
    pub(crate) fn first_from_day(&self, src: &dyn InsightsSource, first_day: i64) -> Option<usize> {
        let mut days = Vec::with_capacity(self.n);
        let day = gitterrace_index::feed::local_day_of;
        if src.chain(&mut |s| days.push(day(s.time, s.offset))) && days.len() == self.n {
            return first_from(days.into_iter(), self.n, first_day);
        }
        let newest = src
            .len()
            .checked_sub(1)
            .map(|i| Rank(u32::try_from(i).unwrap_or(u32::MAX)));
        let days = walk(src, newest).map(|r| {
            src.commit(r).map_or(i64::MIN, |c| {
                gitterrace_index::feed::local_day_of(c.time, c.offset)
            })
        });
        first_from(days, self.n, first_day)
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
            rows_newest_first(
                src,
                walk(src, Some(mark.rank)).take(start - low + 1),
                |r, files| {
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
                },
            );
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

/// The lowest position whose day is `first_day` or later, from the local days of a chain of
/// `n` commits listed newest first. A later commit with an older date does not end the
/// stretch: the result is the first position in the period, whatever follows it.
pub(super) fn first_from(
    days: impl Iterator<Item = i64>,
    n: usize,
    first_day: i64,
) -> Option<usize> {
    let mut found = None;
    for (k, d) in days.enumerate() {
        if d >= first_day {
            found = Some(n - 1 - k);
        }
    }
    found
}

/// Every position the page reads for a range: the shown commits, the chain ends, the
/// commit before the range and before the last ten, and the trend samples.
fn wanted(n: usize, range: Option<(usize, usize)>) -> BTreeSet<usize> {
    let view = View {
        time: false,
        share: false,
        range,
        picked: range.is_some(),
        period: gitterrace_index::Period::All,
        dflt: gitterrace_index::Period::All,
        held: Held::Commits,
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
