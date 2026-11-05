//! The one statistics layer: every number the web pages, the HTTP API and the MCP server show
//! is computed here from cached commit records, under the same [`Options`]. Everything is pure
//! over [`CommitRecord`]s (plus head-tree file lists for "now" questions), so it is
//! unit-testable and the callers only render the returned plain structs.
//!
//! Conventions: records are oldest first as [`crate::Index::records`] returns them. Commit-level
//! numbers count non-merge commits only (a merge's diff against its first parent repeats the
//! branch). Line-level series that must add up exactly (language lines over time) follow the
//! first-parent chain instead, where merge diffs are the only way a branch's lines arrive.

mod accel;
mod axis;
mod code;
mod compare;
mod counted;
pub(crate) mod fasthash;
mod files;
mod filter;
mod format;
mod headfiles;
mod lang;
mod people;
mod period;
mod releases;
mod scope;
mod select;
mod sizes;
mod stats;

use std::sync::{Arc, Mutex, OnceLock};

pub(crate) use gitterrace_git::days_from_civil;
use gitterrace_git::{IgnoreSet, civil_from_days as civil};

use crate::CommitRecord;
use crate::feed::{self, CommitRange, CommitRow, FileRow, IdentId, InsightsSource, PathId, Rank};

pub use accel::{Accel, NoAccel, ReducerKind, ReducerOut};
pub use axis::{BucketUnit, MAX_BUCKETS, TimeAxis};
pub use code::{Frequency, Rework, ReworkPoint, rework_share, rework_verdict};
pub use compare::{Comparison, LangDelta};
pub use counted::{
    CHUNK, Counted, Item, LabelTally, PersonTallies, PersonTally, Reducer, Series, Sizes, Totals,
    When,
};
pub use files::{
    AgeBucket, AgeReport, Coupling, DirOwnership, Hotspot, HotspotReport, Owner, OwnershipReport,
    Touch,
};
pub use filter::{PathFilter, is_generated};
pub use format::{count, thousands};
pub(crate) use headfiles::CACHE_FILE;
pub use headfiles::{HeadFile, LineCache, head_files};
pub use lang::language;
pub use people::{People, Person};
pub use period::{
    Contributor, ContributorReport, Metric, Period, PeriodStats, PeriodSummary, Rhythm, SeriesPoint,
};
pub use releases::{Cadence, Release, release_tags};
pub use scope::{CountContext, Scope, ScopeError, ScopeParams};
pub use select::{OptionParams, Scoped, Selection, under};
pub use sizes::SizeBucket;
pub use stats::{DirChurn, FileChurn, Label, LabelBucket, LabelCount};

pub(crate) const DAY: i64 = 86_400;

/// Derived tables kept for this many (key, options) pairs: the unfiltered pages of a few tips.
const SHARED_SETS: usize = 4;

/// Derived tables of recent unfiltered sources, see [`Insights::shared_source`].
static SHARED: Mutex<Vec<SharedEntry>> = Mutex::new(Vec::new());

/// Which paths and commits count.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per option"
)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Options {
    /// Extra globs (`fnmatch` style, matched against the full path and the file name).
    pub exclude: Vec<String>,
    /// Count lock files and minified output.
    pub include_generated: bool,
    /// Leave out paths that `.gitignore` or `.ignore` rule out.
    pub respect_ignore: bool,
    /// Count whitespace-only and blank-line changes.
    pub count_whitespace: bool,
    /// Count commits by bot accounts in per-file and per-period numbers.
    pub include_bots: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            include_generated: false,
            respect_ignore: true,
            count_whitespace: false,
            include_bots: false,
        }
    }
}

/// Added, removed and rewritten lines of one file change under the active [`Options`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Lines {
    pub add: u32,
    pub del: u32,
    pub rewritten: u32,
}

/// One history read through an [`InsightsSource`], plus everything derived once from it:
/// identities, path filter, caches. Pages read [`Insights::source`], never records.
pub struct Insights<'a> {
    src: Src<'a>,
    pub(crate) opts: Options,
    /// [`feed::reference_time`]: "now" for periods, so old repositories still show data.
    /// Read per instance: it moves with the wall clock, the tables below do not.
    pub(crate) now: i64,
    d: Arc<Derived>,
}

enum Src<'a> {
    Borrowed(&'a dyn InsightsSource),
    #[cfg(any(test, feature = "oracle"))]
    Owned(Box<dyn InsightsSource + 'a>),
}

/// Everything derived from one source under one [`Options`]. Behind an `Arc`, so the
/// insights pages of one tip share it instead of each rebuilding it (the lazy tables cost
/// about a second on a 300k-commit history).
struct Derived {
    filter: PathFilter,
    people: People,
    /// Person index per raw identity id: sized by identities, not by history.
    ident_person: Vec<usize>,
    /// Newest commit time per person (all commits, bots included).
    last_seen: Vec<i64>,
    /// Per path id: whether path statistics leave the path out. Built on first use, each
    /// path decided once, spread over the cores: the ignore rules are the expensive part.
    skip: OnceLock<Vec<bool>>,
    /// Lines changed by each commit over counted files, computed on first use and shared by
    /// every statistic that sums a commit (sizes, rhythm, code frequency, contributors).
    commit_lines: OnceLock<Vec<Lines>>,
    follow: OnceLock<files::Follow>,
}

struct SharedEntry {
    key: String,
    opts: Options,
    /// Address and length of what the tables index into: a hit must be the same set.
    at: (usize, usize),
    d: Arc<Derived>,
}

impl<'a> Insights<'a> {
    /// Over records in memory, through the oracle adapter `feed::MemorySource`: the side
    /// equivalence tests compare the segment set against, never served. `ignore` is the ignore
    /// set of the head revision, see [`IgnoreSet::at`], only used when `opts.respect_ignore` is
    /// set.
    #[cfg(any(test, feature = "oracle"))]
    pub fn new(records: &'a [CommitRecord], opts: Options, ignore: Option<IgnoreSet>) -> Self {
        let src = Src::Owned(Box::new(feed::MemorySource::new(records)));
        let d = Arc::new(Derived::build(src.get(), &opts, ignore));
        Self::with(src, opts, d)
    }

    /// Statistics over `src`.
    pub fn from_source(
        src: &'a dyn InsightsSource,
        opts: Options,
        ignore: Option<IgnoreSet>,
    ) -> Self {
        let d = Arc::new(Derived::build(src, &opts, ignore));
        Self::with(Src::Borrowed(src), opts, d)
    }

    /// [`Insights::from_source`] whose derived tables are shared by every caller passing the
    /// same `key`, options and source, bounded to the last few. `key` must name the set and the
    /// ignore rules, e.g. tip and index generation; `ignore` runs on a miss only.
    pub fn shared_source(
        src: &'a dyn InsightsSource,
        opts: Options,
        key: &str,
        ignore: impl FnOnce() -> Option<IgnoreSet>,
    ) -> Self {
        let at = (std::ptr::from_ref(src).cast::<u8>() as usize, src.len());
        let d = shared_tables(src, &opts, key, at, ignore);
        Self::with(Src::Borrowed(src), opts, d)
    }

    fn with(src: Src<'a>, opts: Options, d: Arc<Derived>) -> Self {
        let now = feed::reference_time(src.get());
        Self { src, opts, now, d }
    }

    /// Drops every shared table set whose key starts with `prefix`. A server releases them with
    /// the records they index into once nobody browses, so an idle process holds neither; the
    /// next page builds them again. Callers holding an `Insights` keep their own `Arc`.
    pub fn release_shared(prefix: &str) {
        if let Ok(mut g) = SHARED.lock() {
            g.retain(|e| !e.key.starts_with(prefix));
        }
    }

    /// Shared table sets whose key starts with `prefix` (a test hook for
    /// [`Insights::release_shared`]).
    pub fn shared_sets(prefix: &str) -> usize {
        SHARED.lock().map_or(0, |g| {
            g.iter().filter(|e| e.key.starts_with(prefix)).count()
        })
    }

    /// True when both share one set of derived tables (a test hook for
    /// [`Insights::shared_source`]).
    pub fn shares_tables_with(&self, other: &Insights<'_>) -> bool {
        Arc::ptr_eq(&self.d, &other.d)
    }

    /// The history every number is computed from.
    pub fn source(&self) -> &dyn InsightsSource {
        self.src.get()
    }

    /// True when path statistics leave `path` out.
    pub fn skips(&self, path: &str) -> bool {
        self.d.filter.skips(path)
    }

    /// True when path statistics leave the head file `path` out. A path the history knows reads
    /// the per-id table, decided once for all paths in parallel; a path it never saw (an
    /// untracked or brand-new name) is decided on its own.
    pub(crate) fn skips_head(&self, path: &str) -> bool {
        self.source()
            .find_path(path)
            .map_or_else(|| self.d.filter.skips(path), |id| self.skips_id(id))
    }

    /// True when path statistics leave path `id` out.
    pub fn skips_id(&self, id: PathId) -> bool {
        self.skip_table()
            .get(id.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    fn skip_table(&self) -> &[bool] {
        self.d.skip.get_or_init(|| {
            let src = self.source();
            let paths: Vec<Arc<str>> = (0..src.paths())
                .map(|i| {
                    src.path(PathId(u32::try_from(i).unwrap_or(u32::MAX)))
                        .unwrap_or_default()
                })
                .collect();
            decide_all(&paths, &self.d.filter)
        })
    }

    /// Lines of one file row under the options, `None` when the path filter leaves it out.
    pub fn row_lines(&self, row: &FileRow) -> Option<Lines> {
        (!self.skips_id(row.path)).then(|| self.row_lines_unfiltered(row))
    }

    /// Lines of one file row under the whitespace option, whether or not the path filter
    /// would skip it.
    pub fn row_lines_unfiltered(&self, row: &FileRow) -> Lines {
        let (add, del, rewritten) = if self.opts.count_whitespace {
            (row.add, row.del, row.rewritten)
        } else {
            row.code()
        };
        Lines {
            add,
            del,
            rewritten,
        }
    }

    /// Person index of a raw identity.
    pub fn person_of(&self, id: IdentId) -> usize {
        self.d.ident_person.get(id.0 as usize).copied().unwrap_or(0)
    }

    /// Non-merge commit by someone who counts under the options.
    pub fn counted_row(&self, c: &CommitRow) -> bool {
        !c.merge && (self.opts.include_bots || !self.d.people.list[self.person_of(c.ident)].bot)
    }

    /// [`Insights::counted_row`] by rank.
    pub fn counted_rank(&self, rank: Rank) -> bool {
        self.source()
            .commit(rank)
            .is_some_and(|c| self.counted_row(&c))
    }

    /// Lines changed by commit `rank` over counted files.
    pub fn commit_sums(&self, rank: Rank) -> Lines {
        self.d
            .commit_lines
            .get_or_init(|| {
                let src = self.source();
                let all: Vec<Rank> = (0..src.len()).map(rank_of).collect();
                src.rows_for(all)
                    .map(|(_, rows)| {
                        rows.iter().filter_map(|r| self.row_lines(r)).fold(
                            Lines::default(),
                            |mut out, l| {
                                out.add += l.add;
                                out.del += l.del;
                                out.rewritten += l.rewritten;
                                out
                            },
                        )
                    })
                    .collect()
            })
            .get(rank.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    /// Buckets for a time chart of `period` (`None` and `All` span the history).
    pub(crate) fn axis(&self, period: Option<Period>) -> TimeAxis {
        let src = self.source();
        let first = src
            .commits_in(CommitRange::All)
            .map(|c| feed::local_day_of(c.time, c.offset))
            .min();
        match (first, reference_day(src)) {
            (Some(first), Some(last)) => TimeAxis::for_period(period, first, last),
            _ => TimeAxis::for_period(period, 1, 0),
        }
    }

    /// Bucket width of every time chart for `period`.
    pub fn bucket_unit(&self, period: Period) -> BucketUnit {
        self.axis(Some(period)).unit()
    }

    pub(crate) fn follow(&self) -> &files::Follow {
        self.d.follow.get_or_init(|| self.build_follow())
    }
}

impl Src<'_> {
    fn get(&self) -> &dyn InsightsSource {
        match self {
            Self::Borrowed(s) => *s,
            #[cfg(any(test, feature = "oracle"))]
            Self::Owned(s) => s.as_ref(),
        }
    }
}

impl Derived {
    fn build(src: &dyn InsightsSource, opts: &Options, ignore: Option<IgnoreSet>) -> Self {
        // One pass: commits and newest time per raw identity, folded into people below.
        let mut per_ident = vec![0usize; src.idents()];
        let mut newest = vec![i64::MIN; src.idents()];
        for c in src.commits_in(CommitRange::All) {
            let i = c.ident.0 as usize;
            if let (Some(n), Some(t)) = (per_ident.get_mut(i), newest.get_mut(i)) {
                *n += 1;
                *t = (*t).max(c.time);
            }
        }
        let names: Vec<(Arc<str>, Arc<str>)> = (0..per_ident.len())
            .map(|i| {
                src.ident(IdentId(u32::try_from(i).unwrap_or(u32::MAX)))
                    .unwrap_or_default()
            })
            .collect();
        let people = People::from_counts(
            names
                .iter()
                .zip(&per_ident)
                .filter(|(_, n)| **n > 0)
                .map(|((a, e), n)| ((&**a, &**e), *n)),
        );
        let ident_person: Vec<usize> = names.iter().map(|(a, e)| people.index_of(a, e)).collect();
        let mut last_seen = vec![i64::MIN; people.list.len()];
        for (i, &t) in newest.iter().enumerate() {
            let p = ident_person.get(i).copied().unwrap_or(0);
            if let Some(s) = last_seen.get_mut(p) {
                *s = (*s).max(t);
            }
        }
        Self {
            filter: PathFilter::new(opts, ignore),
            people,
            ident_person,
            last_seen,
            skip: OnceLock::new(),
            commit_lines: OnceLock::new(),
            follow: OnceLock::new(),
        }
    }
}

fn shared_tables(
    src: &dyn InsightsSource,
    opts: &Options,
    key: &str,
    at: (usize, usize),
    ignore: impl FnOnce() -> Option<IgnoreSet>,
) -> Arc<Derived> {
    let hit = SHARED.lock().ok().and_then(|g| {
        g.iter()
            .find(|e| e.key == key && e.opts == *opts && e.at == at)
            .map(|e| Arc::clone(&e.d))
    });
    hit.unwrap_or_else(|| {
        let d = Arc::new(Derived::build(src, opts, ignore()));
        if let Ok(mut g) = SHARED.lock() {
            g.retain(|e| !(e.key == key && e.opts == *opts));
            if g.len() >= SHARED_SETS {
                g.remove(0);
            }
            g.push(SharedEntry {
                key: key.to_owned(),
                opts: opts.clone(),
                at,
                d: Arc::clone(&d),
            });
        }
        d
    })
}

/// Decides every path once, spread over the cores.
fn decide_all(paths: &[Arc<str>], filter: &PathFilter) -> Vec<bool> {
    decide_with(paths, |p| filter.decide(p))
}

/// [`decide_all`] with the decision passed in. A panic in any chunk is propagated: dropping
/// the chunk would shorten the table and shift every later path's decision.
fn decide_with(paths: &[Arc<str>], decide: impl Fn(&str) -> bool + Sync) -> Vec<bool> {
    let decide = &decide;
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let chunk = paths.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        #[expect(
            clippy::needless_collect,
            reason = "all threads must be spawned before the first join, or they would run one after another"
        )]
        let parts: Vec<_> = paths
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().map(|p| decide(p)).collect::<Vec<_>>()))
            .collect();
        let skip: Vec<bool> = parts
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect();
        debug_assert_eq!(skip.len(), paths.len(), "skip table out of step with paths");
        skip
    })
}

pub(crate) fn rank_of(i: usize) -> Rank {
    Rank(u32::try_from(i).unwrap_or(u32::MAX))
}

/// Local day of the commit that sets [`feed::reference_time`]: fixed-period axes end on it.
/// The newest commit not after "now" (the last of equals), else the oldest (the first of
/// equals), as `max_by_key` and `min_by_key` picked over the records.
pub(crate) fn reference_day(src: &dyn InsightsSource) -> Option<i64> {
    scope::reference_day_at(src, feed::reference_time(src))
}

/// The record that sets [`crate::reference_time`]: the record-side oracle of
/// [`reference_day`] in the axis tests.
#[cfg(test)]
pub(crate) fn reference_record(records: &[CommitRecord]) -> Option<&CommitRecord> {
    let now = crate::reference_time(records);
    records
        .iter()
        .filter(|r| r.time <= now)
        .max_by_key(|r| r.time)
        .or_else(|| records.iter().min_by_key(|r| r.time))
}

/// Author time shifted into the author's own time zone.
pub const fn local_time(r: &CommitRecord) -> i64 {
    feed::local_time_of(r.time, r.offset)
}

/// Days since 1970-01-01 in the author's own time zone, [`feed::local_day_of`] of a record.
pub const fn local_day(r: &CommitRecord) -> i64 {
    feed::local_day_of(r.time, r.offset)
}

/// Monday is 0. Day 0 (1970-01-01) was a Thursday.
pub fn weekday(days: i64) -> usize {
    usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0)
}

pub(crate) fn format_date(days: i64) -> String {
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Bus factor: the fewest contributors whose shares, largest first, hold at least half.
///
/// Shares in any order; zero when there are none.
/// The one definition every surface (insights, API, MCP, cards) uses.
pub fn bus_factor(shares: impl IntoIterator<Item = f64>) -> usize {
    let mut v: Vec<f64> = shares.into_iter().filter(|s| *s > 0.0).collect();
    v.sort_by(|a, b| b.total_cmp(a));
    let total: f64 = v.iter().sum();
    let mut acc = 0.0;
    for (i, s) in v.iter().enumerate() {
        acc += s;
        if acc * 2.0 >= total {
            return i + 1;
        }
    }
    0
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::{CommitRecord, FileStat};

    // 2024-01-01 is a Monday.
    pub const MON: i64 = 1_704_067_200;
    pub const DAYS: i64 = 86_400;

    pub fn file(path: &str, add: u32, del: u32, lines: u32) -> FileStat {
        FileStat {
            path: path.into(),
            additions: add,
            deletions: del,
            lines,
            ..FileStat::default()
        }
    }

    /// Four commits in a line: a, b, c, d.
    pub fn chain() -> Vec<CommitRecord> {
        let mut v = vec![
            rec("a", "Ann", 0, vec![file("x.rs", 1, 0, 1)]),
            rec("b", "Ann", 10, vec![file("x.rs", 30, 0, 31)]),
            rec(
                "c",
                "Bob",
                30,
                vec![file("y.py", 300, 100, 200), file("x.rs", 0, 3, 28)],
            ),
            rec("d", "Bob", 40, vec![file("y.py", 3000, 0, 3200)]),
        ];
        for i in 1..v.len() {
            v[i].parents = vec![v[i - 1].id.clone()];
        }
        v
    }

    pub fn rec(id: &str, who: &str, day: i64, files: Vec<FileStat>) -> CommitRecord {
        CommitRecord {
            id: id.into(),
            author: who.into(),
            email: format!("{}@x.io", who.to_lowercase()),
            time: MON + day * DAYS + 3600 * 12,
            files,
            ..CommitRecord::default()
        }
    }
}

#[cfg(test)]
mod tests;
