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
mod figure;
pub(crate) mod files;
mod filter;
mod fold;
mod format;
mod headfiles;
mod lang;
mod lines;
mod people;
mod period;
mod releases;
mod scope;
mod select;
mod shared;
mod sizes;
mod stats;

use std::sync::{Arc, Mutex, OnceLock};

use gitterrace_git::IgnoreSet;
pub(crate) use gitterrace_git::days_from_civil;

use crate::CommitRecord;
use crate::feed::{self, CommitRange, CommitRow, FileRow, IdentId, InsightsSource, PathId, Rank};

pub use accel::{Accel, NoAccel, ReducerKind, ReducerOut};
pub use axis::{BucketUnit, MAX_BUCKETS, TimeAxis};
pub use code::{
    Frequency, FrequencyTally, LineTotals, Rework, ReworkPoint, rework_share, rework_verdict,
};
pub use compare::{Comparison, LangDelta, RawCommit};
pub use counted::{
    CHUNK, Counted, Item, LabelTally, PersonTallies, PersonTally, Reducer, Series, Sizes, Totals,
    When,
};
pub use figure::{EveryTotals, Figure, Population};
pub use files::{
    AgeBucket, AgeReport, Coupling, DirOwnership, Hotspot, HotspotReport, Owner, OwnershipReport,
    Touch,
};
pub use filter::{PathFilter, is_generated};
pub use fold::ActivityFold;
pub use format::{count, thousands};
pub(crate) use headfiles::CACHE_FILE;
pub use headfiles::{HeadFile, LineCache, head_files};
pub use lang::{language, language_key};
pub use people::{People, Person};
pub use period::{
    Contributor, ContributorReport, ContributorTally, Metric, Period, PeriodStats, PeriodSummary,
    Rhythm, SeriesPoint,
};
pub use releases::{Cadence, Release, release_tags};
pub use scope::{CountContext, Scope, ScopeError, ScopeParams, parse_day};
pub use select::{OptionParams, Scoped, Selection, under};
pub use sizes::SizeBucket;
pub use stats::{DirChurn, DirTally, FileChurn, FileTally, Label, LabelBucket, LabelCount};

pub(crate) use gitterrace_git::DAY;

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
    /// Count merge commits as commits. A merge never adds lines or files.
    pub include_merges: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            include_generated: false,
            respect_ignore: true,
            count_whitespace: false,
            include_bots: false,
            include_merges: false,
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
    commit_lines: OnceLock<lines::CommitLines>,
    follow: OnceLock<files::Follow>,
    /// The period anchors of this history for the `now` they were read at, read once per
    /// `now` instead of once per section (a scan of every commit column).
    context: Mutex<Option<(i64, CountContext)>>,
    /// Newest author time of the history, so [`feed::reference_time`] needs no scan while the
    /// clock is past it (the usual case; a future-dated commit scans as before).
    newest: OnceLock<i64>,
    /// The stored tip touches grouped by tip name, `None` without a tip file: what every
    /// per-file section slices (see [`files::TipIndex`]). Released with these tables on idle.
    tip: OnceLock<Option<files::TipIndex>>,
    /// Path ids of names looked up by the per-file pages (the head tree, 64k on rust), so a
    /// page and its sections pay each lookup once per set, not once per call.
    ids: Mutex<std::collections::HashMap<Arc<str>, Option<PathId>>>,
    /// Walks of a counted set over these tables, for the tests that count work per page.
    #[cfg(any(test, feature = "oracle"))]
    walks: std::sync::atomic::AtomicUsize,
    /// The walks of those that read file rows.
    #[cfg(any(test, feature = "oracle"))]
    row_walks: std::sync::atomic::AtomicUsize,
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
        let d = shared::tables(src, &opts, key, at, ignore);
        Self::with(Src::Borrowed(src), opts, d)
    }

    fn with(src: Src<'a>, opts: Options, d: Arc<Derived>) -> Self {
        let newest = *d.newest.get_or_init(|| {
            src.get()
                .commits_in(CommitRange::All)
                .map(|c| c.time)
                .max()
                .unwrap_or(0)
        });
        // The scan's answer when no commit is past the clock: the newest time.
        let now = if newest <= feed::Clock::now(&feed::SystemClock) && newest >= 0 {
            newest
        } else {
            feed::reference_time(src.get())
        };
        Self { src, opts, now, d }
    }

    /// Drops every shared table set whose key starts with `prefix`. A server releases them with
    /// the records they index into once nobody browses, so an idle process holds neither; the
    /// next page builds them again. Callers holding an `Insights` keep their own `Arc`.
    pub fn release_shared(prefix: &str) {
        if let Ok(mut g) = shared::SHARED.lock() {
            g.retain(|e| !e.key.starts_with(prefix));
        }
    }

    /// Shared table sets whose key starts with `prefix` (a test hook for
    /// [`Insights::release_shared`]).
    #[cfg(any(test, feature = "oracle"))]
    pub fn shared_sets(prefix: &str) -> usize {
        shared::SHARED.lock().map_or(0, |g| {
            g.iter().filter(|e| e.key.starts_with(prefix)).count()
        })
    }

    /// True when both share one set of derived tables (a test hook for
    /// [`Insights::shared_source`]).
    #[cfg(test)]
    pub(crate) fn shares_tables_with(&self, other: &Insights<'_>) -> bool {
        Arc::ptr_eq(&self.d, &other.d)
    }

    /// The history every number is computed from.
    pub fn source(&self) -> &dyn InsightsSource {
        self.src.get()
    }

    /// Counted-set walks over these tables so far, shared with every `Insights` of them: what
    /// a page costs, counted where a timing would be noise.
    #[cfg(any(test, feature = "oracle"))]
    pub fn walks(&self) -> usize {
        self.d.walks.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The [`Insights::walks`] that read file rows.
    #[cfg(any(test, feature = "oracle"))]
    pub fn row_walks(&self) -> usize {
        self.d.row_walks.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(any(test, feature = "oracle"))]
    fn count_walk(&self, rows: bool) {
        self.d
            .walks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if rows {
            self.d
                .row_walks
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[cfg(not(any(test, feature = "oracle")))]
    const fn count_walk(&self, _rows: bool) {}

    /// True when path statistics leave `path` out.
    pub fn skips(&self, path: &str) -> bool {
        self.d.filter.skips(path)
    }

    /// True when path statistics leave the head file `path` out. A path the history knows reads
    /// the per-id table, decided once for all paths in parallel; a path it never saw (an
    /// untracked or brand-new name) is decided on its own.
    pub(crate) fn skips_head(&self, path: &str) -> bool {
        self.head_id(path)
            .map_or_else(|| self.d.filter.skips(path), |id| self.skips_id(id))
    }

    /// [`InsightsSource::path_id`] remembered per set: head names are asked for by every
    /// per-file section. A storage error is recorded by the source and not remembered.
    pub(crate) fn head_id(&self, path: &str) -> Option<PathId> {
        if let Ok(g) = self.d.ids.lock()
            && let Some(id) = g.get(path)
        {
            return *id;
        }
        let id = self.source().find_path(path);
        let Ok(id) = id else {
            return None;
        };
        if let Ok(mut g) = self.d.ids.lock() {
            g.insert(path.into(), id);
        }
        id
    }

    /// Remembers the ids of `paths` in one pass (the warm-up gives it the head tree).
    pub fn warm_paths<'p>(&self, paths: impl IntoIterator<Item = &'p str>) {
        for p in paths {
            let _ = self.head_id(p);
        }
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

    /// Commit by someone who counts under the options: a merge only with `include_merges`.
    pub fn counted_row(&self, c: &CommitRow) -> bool {
        (!c.merge || self.opts.include_merges)
            && (self.opts.include_bots || !self.d.people.list[self.person_of(c.ident)].bot)
    }

    /// Lines changed by commit `c` over counted files. Takes the row the caller holds: a
    /// lookup by rank per commit thrashed the block cache on a large history.
    pub fn commit_sums(&self, c: &CommitRow) -> Lines {
        if c.merge {
            return Lines::default();
        }
        let rank = c.rank;
        self.commit_lines()
            .get(rank.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    /// The tip index, built on first use (the server's ready warm-up asks for it).
    pub(crate) fn tip_index(&self) -> Option<&files::TipIndex> {
        self.d
            .tip
            .get_or_init(|| files::TipIndex::build(self.source()))
            .as_ref()
    }

    /// Builds the tables the per-file pages share, so the first visitor after the warm-up
    /// does not pay them: the path filter's decisions, the anchors and the tip index.
    pub fn warm_files(&self) {
        let _ = self.skips_id(PathId(0));
        let _ = self.context();
        let _ = self.tip_index();
        let _ = self.commit_lines();
    }

    pub(crate) fn follow(&self) -> &files::Follow {
        self.d
            .follow
            .get_or_init(|| files::Follow::build(self.source()))
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

/// Table sets built per source address, for the tests that count work per page.
#[cfg(any(test, feature = "oracle"))]
static BUILDS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// [`Derived`] sets built so far over the source at `src`: every fresh [`Insights`] of a
/// source that the shared tables already hold builds one more.
#[cfg(any(test, feature = "oracle"))]
pub fn table_builds(src: &dyn InsightsSource) -> usize {
    let at = std::ptr::from_ref(src).cast::<u8>() as usize;
    BUILDS
        .lock()
        .ok()
        .and_then(|g| g.iter().find(|(a, _)| *a == at).map(|(_, n)| *n))
        .unwrap_or(0)
}

impl Derived {
    fn build(src: &dyn InsightsSource, opts: &Options, ignore: Option<IgnoreSet>) -> Self {
        #[cfg(any(test, feature = "oracle"))]
        if let Ok(mut g) = BUILDS.lock() {
            let at = std::ptr::from_ref(src).cast::<u8>() as usize;
            match g.iter_mut().find(|(a, _)| *a == at) {
                Some(e) => e.1 += 1,
                None => g.push((at, 1)),
            }
        }
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
            context: Mutex::default(),
            newest: OnceLock::new(),
            tip: OnceLock::new(),
            ids: Mutex::default(),
            #[cfg(any(test, feature = "oracle"))]
            walks: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(any(test, feature = "oracle"))]
            row_walks: std::sync::atomic::AtomicUsize::new(0),
        }
    }
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
            reason = "all threads must be spawned before the first join, or they would run one \
                after another"
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
#[cfg(test)]
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

pub(crate) use gitterrace_git::ymd as format_date;

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
pub(crate) mod fixtures;
#[cfg(test)]
mod tests;
