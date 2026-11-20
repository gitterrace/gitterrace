//! The one seam every insights page reads through: [`InsightsSource`].
//!
//! Pages read the segment files (`segments.rs`, `disk.rs`). The memory adapter over records
//! (`memory.rs`) exists only as the oracle the equivalence tests compare the segments against:
//! it is compiled for tests and the `oracle` feature, which only dev-dependencies enable.
//!
//! Contract:
//!
//! - Commits are addressed by [`Rank`], the position in the canonical order of the whole
//!   reachable set, oldest first. Ranks are dense: `0..len()`. A source over several unmerged
//!   segments maps its segment-local ordinals to ranks itself, a page never sees an ordinal.
//! - Ids ([`PathId`], [`IdentId`], [`LabelId`]) are stable for the life of one source value.
//! - Identities are served with the mailmap of the shown revision already applied: two raw
//!   identities the mailmap joins come back with equal strings (they may keep two ids).
//!   Merging people beyond the mailmap ([`crate::People`]) and the bot bit stay in the
//!   insights core, as do every option (whitespace, path filter, bots). A source serves raw
//!   facts, both line columns (raw and code), and never applies an option.
//! - Every iterator is streaming: memory is bounded by one block or one record, never by the
//!   history. Results come in ascending rank order unless stated otherwise.
//! - Object safe: pages take `&dyn InsightsSource`.
//! - Storage errors are never absence: a finder (`find_*`) returns `Err`, and every call, a
//!   finder's error included, records the error for [`InsightsSource::failure`]. Pages and
//!   answers read a source only through [`Checked::run`], which refuses anything built while
//!   an error happened.

pub(crate) mod disk;
mod fresh;
#[cfg(any(test, feature = "oracle"))]
mod memory;
mod reach;
pub(crate) mod segments;

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

pub use crate::seg::{IdentId, LabelId, Oid, PathId, Rank};
pub use crate::tip::TipTouch;
#[cfg(any(test, feature = "oracle"))]
pub use disk::testing as disk_testing;
pub use disk::{DiskSource, IdentMap, RowWriter, block_cache_default};
pub use fresh::FRESH_CAP;
#[cfg(any(test, feature = "oracle"))]
pub use memory::{IdentPairs, MemorySource};
pub use reach::{Reach, RevView, rev_view};
pub use segments::SegmentSource;

/// Where "now" comes from. Serving reads [`SystemClock`]; tests pass a [`FixedClock`], so no
/// count depends on when a test runs.
pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

/// This machine's clock (frozen by `GITTERRACE_TEST_NOW` for the equivalence oracle).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        crate::wall_clock()
    }
}

/// A clock stopped at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now(&self) -> i64 {
        self.0
    }
}

/// "Now" for period windows over a source: the newest author time not after the wall clock.
pub fn reference_time(src: &dyn InsightsSource) -> i64 {
    reference_time_at(src, &SystemClock)
}

/// [`reference_time`] with the clock given.
pub fn reference_time_at(src: &dyn InsightsSource, clock: &dyn Clock) -> i64 {
    let wall = clock.now();
    src.commits_in(CommitRange::All).map(|c| c.time).filter(|t| *t <= wall).max().unwrap_or(0)
}

/// [`crate::local_time`] from the columns.
pub const fn local_time_of(time: i64, offset: i32) -> i64 {
    time.saturating_add(offset as i64)
}

/// [`crate::local_day`] from the columns.
pub const fn local_day_of(time: i64, offset: i32) -> i64 {
    time.saturating_add(offset as i64).div_euclid(crate::insights::DAY)
}

/// Author x day rows over `commits` (merges skipped), for views that fold their own rows.
pub(crate) fn fold_author_day(
    commits: impl Iterator<Item = CommitRow>,
    days: &Range<i64>,
    over: &SumOverflow,
) -> Stream<'static, AuthorDayRow> {
    let mut m: BTreeMap<(i64, IdentId), (u64, Sums)> = BTreeMap::new();
    for c in commits.filter(|c| !c.merge) {
        let day = local_day_of(c.time, c.offset);
        if days.contains(&day) {
            let e = m.entry((day, c.ident)).or_default();
            e.0 += 1;
            over.note(e.1.merge(&c.sums));
        }
    }
    Box::new(m.into_iter().map(|((day, ident), (commits, sums))| AuthorDayRow {
        day,
        ident,
        commits,
        sums,
    }))
}

/// Path x author rows over the rows `each` hands out (merges skipped by the caller), keeping
/// the paths `keep` accepts.
pub(crate) fn fold_path_author(
    over: &SumOverflow,
    mut keep: impl FnMut(PathId) -> bool,
    each: impl FnOnce(&mut dyn FnMut(&CommitRow, &[FileRow])),
) -> Stream<'static, PathAuthorRow> {
    let mut m: BTreeMap<(PathId, IdentId), (u64, Sums)> = BTreeMap::new();
    each(&mut |c, files| {
        if c.merge {
            return;
        }
        for f in files.iter().filter(|f| keep(f.path)) {
            let e = m.entry((f.path, c.ident)).or_default();
            e.0 += 1;
            over.note(e.1.add_row(f));
        }
    });
    Box::new(m.into_iter().map(|((path, ident), (touches, sums))| PathAuthorRow {
        path,
        ident,
        touches,
        sums,
    }))
}

pub type Stream<'s, T> = Box<dyn Iterator<Item = T> + 's>;

/// Line sums of one change set: raw, and with whitespace-only changes ignored ("code").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sums {
    pub add: u64,
    pub del: u64,
    pub rewritten: u64,
    pub code_add: u64,
    pub code_del: u64,
    pub code_rewritten: u64,
}

impl Sums {
    pub(crate) fn add_file(&mut self, f: &crate::FileStat) {
        let c = f.code_stat();
        self.add += u64::from(f.additions);
        self.del += u64::from(f.deletions);
        self.rewritten += u64::from(f.rewritten);
        self.code_add += u64::from(c.additions);
        self.code_del += u64::from(c.deletions);
        self.code_rewritten += u64::from(c.rewritten);
    }

    /// Adds one file row, or `None` when a sum passes u64 (`self` unchanged then). Rows read
    /// from a file can repeat without bound across commits, so this is checked as
    /// [`Sums::merge`] is.
    #[must_use]
    pub fn add_row(&mut self, f: &FileRow) -> Option<()> {
        let c = f.code();
        self.merge(&Self {
            add: u64::from(f.add),
            del: u64::from(f.del),
            rewritten: u64::from(f.rewritten),
            code_add: u64::from(c.0),
            code_del: u64::from(c.1),
            code_rewritten: u64::from(c.2),
        })
    }

    /// Adds `o`, or `None` when a sum passes u64: only stored counts get there, and the caller
    /// reports the cache unreadable then. `self` is unchanged on `None`.
    #[must_use]
    pub const fn merge(&mut self, o: &Self) -> Option<()> {
        let (Some(add), Some(del), Some(rewritten)) = (
            self.add.checked_add(o.add),
            self.del.checked_add(o.del),
            self.rewritten.checked_add(o.rewritten),
        ) else {
            return None;
        };
        let (Some(code_add), Some(code_del), Some(code_rewritten)) = (
            self.code_add.checked_add(o.code_add),
            self.code_del.checked_add(o.code_del),
            self.code_rewritten.checked_add(o.code_rewritten),
        ) else {
            return None;
        };
        *self = Self { add, del, rewritten, code_add, code_del, code_rewritten };
        Some(())
    }
}

/// One commit's columns. No strings: ask [`InsightsSource::commit_id`] and the string lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitRow {
    pub rank: Rank,
    /// Raw identity (mailmap applied to its strings, see the module contract).
    pub ident: IdentId,
    pub time: i64,
    /// Author time zone offset in seconds, as [`crate::CommitRecord::offset`].
    pub offset: i32,
    pub merge: bool,
    pub kind: Option<LabelId>,
    pub scope: Option<LabelId>,
    pub files: u32,
    pub sums: Sums,
}

/// One file of one commit, as [`crate::FileStat`] with ids for strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileRow {
    pub path: PathId,
    /// Previous path when renamed in this commit.
    pub old: Option<PathId>,
    pub add: u32,
    pub del: u32,
    pub lines: u32,
    pub rewritten: u32,
    /// (add, del, rewritten) ignoring whitespace, `None` when equal to the raw counts.
    pub code: Option<(u32, u32, u32)>,
    pub binary: bool,
    pub deleted: bool,
}

impl FileRow {
    /// Whitespace-ignoring counts, the raw ones when nothing differs.
    pub fn code(&self) -> (u32, u32, u32) {
        self.code.unwrap_or((self.add, self.del, self.rewritten))
    }
}

/// Which commits [`InsightsSource::commits_in`] yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitRange {
    All,
    Ranks(Range<u32>),
    /// Author time `[from, to)` in epoch seconds (UTC, as [`gitterrace_types::Period::window`]).
    Time(Range<i64>),
}

/// Author x local day aggregate, merges left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorDayRow {
    /// Days since 1970-01-01 in the author's time zone ([`crate::local_day`]).
    pub day: i64,
    pub ident: IdentId,
    pub commits: u64,
    pub sums: Sums,
}

/// Path x author aggregate over the whole history, merges left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathAuthorRow {
    pub path: PathId,
    pub ident: IdentId,
    pub touches: u64,
    pub sums: Sums,
}

/// What [`InsightsSource::postings`] lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Posting {
    /// Non-merge commits touching the path under this exact name (no rename follow).
    Path(PathId),
    /// Every commit (merges included) by this raw identity.
    Author(IdentId),
}

/// One posting entry: the commit and, for a path, the index of its file row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit {
    pub rank: Rank,
    /// File row index inside the commit, 0 for author postings.
    pub row: u32,
}

/// The file at `row` of commit `rank` was named `old` before and `new` after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RenameEdge {
    pub rank: Rank,
    pub row: u32,
    pub old: PathId,
    pub new: PathId,
}

/// The next [`InsightsSource::instance`] id, from 1: unique for the life of the process.
#[cfg(any(test, feature = "oracle"))]
pub(crate) fn next_instance() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Read access to one indexed history. See the module docs for the contract.
pub trait InsightsSource: Send + Sync {
    /// Commits indexed (`commits_indexed` on the pages).
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn commit(&self, rank: Rank) -> Option<CommitRow>;

    /// Commit columns in `range`, ascending rank. A time range is a filter over ranks: author
    /// times are not monotonic in canonical order.
    fn commits_in(&self, range: CommitRange) -> Stream<'_, CommitRow>;

    /// File rows of the given commits, in the order given (callers pass ascending ranks).
    fn rows_for(&self, commits: Vec<Rank>) -> Stream<'_, (Rank, Vec<FileRow>)>;

    /// Commit id as hex (or whatever id the record carries).
    fn commit_id(&self, rank: Rank) -> Option<Arc<str>>;

    /// Parent ids, first parent first, as written (a parent may be outside the set).
    fn parent_ids(&self, rank: Rank) -> Vec<Arc<str>>;

    /// The commit with this id: `Ok(None)` when it is not indexed, `Err` when the store could
    /// not tell. A caller that walks git when a commit is absent must not walk on an error.
    fn find_commit(&self, id: &str) -> Result<Option<Rank>, SourceError>;

    /// Author x day rows with `day` in `days`, ascending (day, ident).
    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow>;

    /// Path x author rows whose path is `prefix` or under it (`""` for all), ascending
    /// (path, ident).
    fn agg_path_author(&self, prefix: &str) -> Stream<'_, PathAuthorRow>;

    fn postings(&self, key: Posting) -> Stream<'_, Hit>;

    /// Every rename edge, ascending rank. Follow walks them newest first.
    fn rename_edges(&self) -> Stream<'_, RenameEdge>;

    fn path(&self, id: PathId) -> Option<Arc<str>>;
    /// The id of `path`: `Ok(None)` when no indexed commit names it, `Err` when the store could
    /// not tell.
    fn find_path(&self, path: &str) -> Result<Option<PathId>, SourceError>;
    /// Number of path ids, so per-path bit vectors can be sized.
    fn paths(&self) -> usize;

    fn ident(&self, id: IdentId) -> Option<(Arc<str>, Arc<str>)>;
    fn idents(&self) -> usize;

    fn label(&self, id: LabelId) -> Option<Arc<str>>;
    /// The id of a kind or scope: `Ok(None)` when no commit carries it.
    fn find_label(&self, name: &str) -> Result<Option<LabelId>, SourceError>;

    /// [`InsightsSource::find_commit`] inside a build that [`Checked::run`] guards: an error is
    /// already recorded for [`InsightsSource::failure`], so the build may treat it as absent.
    /// Code that acts on absence outside such a build (a git walk instead) calls the finder.
    fn commit_of(&self, id: &str) -> Option<Rank> {
        self.find_commit(id).unwrap_or(None)
    }

    /// [`InsightsSource::find_path`] inside a guarded build, see [`InsightsSource::commit_of`].
    fn path_id(&self, path: &str) -> Option<PathId> {
        self.find_path(path).unwrap_or(None)
    }

    /// [`InsightsSource::find_label`] inside a guarded build, see [`InsightsSource::commit_of`].
    fn label_id(&self, name: &str) -> Option<LabelId> {
        self.find_label(name).unwrap_or(None)
    }

    /// The first storage error any call met, if any. Lookups and streams answer absent or
    /// short after one, so nothing built from this source may be shown: [`Checked::run`] is how
    /// a page or an answer reads a source, and it refuses the result then. No default, so a new
    /// source has to say where its errors go.
    fn failure(&self) -> Option<SourceError>;

    /// An id unique to this source for the life of the process, 0 when it has none: the
    /// tests that count work per source key by it, since an address is reused once freed.
    #[cfg(any(test, feature = "oracle"))]
    fn instance(&self) -> u64 {
        0
    }

    /// Hands every commit's file rows keyed by the file's name at the indexed tip to `each`,
    /// ascending rank, merges with none, when the index stored them for exactly this set (see
    /// `crate::tip`). `false` when it did not: the caller reads the rows then. A view whose
    /// ranks are not the stored set's keeps the default.
    fn tip_touches(&self, _each: &mut dyn FnMut(Rank, &[TipTouch])) -> bool {
        false
    }

    /// Whether [`InsightsSource::tip_touches`] would stream, without reading it.
    fn has_tip(&self) -> bool {
        false
    }
}

/// Set when a source summed stored counts past u64. The sum is then wrong whichever way it is
/// rounded, so the source reports the cache unreadable ([`InsightsSource::failure`]) instead of
/// answering with a saturated or wrapped number.
#[derive(Debug, Default)]
pub(crate) struct SumOverflow(std::sync::atomic::AtomicBool);

impl SumOverflow {
    pub(crate) const fn new() -> Self {
        Self(std::sync::atomic::AtomicBool::new(false))
    }
    /// Notes the result of a checked add: `None` sets the flag.
    pub(crate) fn note(&self, r: Option<()>) {
        if r.is_none() {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The unreadable-cache error once an add overflowed.
    pub(crate) fn failure(&self) -> Option<SourceError> {
        self.0
            .load(std::sync::atomic::Ordering::Relaxed)
            .then(|| SourceError(crate::seg::agg::SUM_OVER.to_owned()))
    }
}

/// A storage error a source met while answering. The cache it came from is unreadable: the
/// caller rebuilds it and never shows what was built over it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("insights cache unreadable: {0}")]
pub struct SourceError(pub String);

impl SourceError {
    /// The storage error inside `e`, when an open or a walk over the set failed on one (the
    /// index wraps it in an `io::Error` there). Such an error means: rebuild, never show.
    pub fn of(e: &std::io::Error) -> Option<Self> {
        e.get_ref()?.downcast_ref::<Self>().cloned()
    }

    /// `e` as an `io::Error` that [`SourceError::of`] finds again.
    pub(crate) fn io(e: impl std::fmt::Display) -> std::io::Error {
        std::io::Error::other(Self(e.to_string()))
    }
}

/// A source that can only be read through [`Checked::run`].
///
/// Whatever the closure builds is handed out only when the source met no storage error
/// meanwhile. A page that skips the check does not compile, since there is no other way to the
/// source.
#[must_use = "a source is read through Checked::run, which checks it for storage errors"]
pub struct Checked<S: ?Sized> {
    src: Arc<S>,
}

impl<S: InsightsSource + ?Sized> Checked<S> {
    pub const fn new(src: Arc<S>) -> Self {
        Self { src }
    }

    /// Builds `f` over the source and hands the result out only when no storage error happened
    /// before or during it. Consumes the handle: one check per build.
    ///
    /// # Errors
    /// The first storage error the source met, before or during `f`.
    pub fn run<T>(self, f: impl FnOnce(&Arc<S>) -> T) -> Result<T, SourceError> {
        if let Some(e) = self.src.failure() {
            return Err(e);
        }
        let out = f(&self.src);
        self.src.failure().map_or(Ok(out), Err)
    }

    /// Commits in the set, a number that is never read from a block.
    pub fn len(&self) -> usize {
        self.src.len()
    }

    pub fn is_empty(&self) -> bool {
        self.src.is_empty()
    }

    /// The first storage error the source met, without reading anything: a holder drops a
    /// failed source and rebuilds the cache.
    pub fn failure(&self) -> Option<SourceError> {
        self.src.failure()
    }

    /// The same source, checked again on its own.
    pub fn share(&self) -> Self {
        Self { src: Arc::clone(&self.src) }
    }

    /// The same handle over the trait object, so sources of different kinds share one slot.
    pub fn erased(self) -> Checked<dyn InsightsSource>
    where
        S: Sized + 'static,
    {
        let src: Arc<dyn InsightsSource> = self.src;
        Checked { src }
    }

    /// Whether `other` is this very source (the tip's tables are shared by address).
    pub fn holds(&self, other: &Arc<S>) -> bool {
        Arc::ptr_eq(&self.src, other)
    }

    /// The source without the check, for tests that probe it call by call.
    #[cfg(any(test, feature = "oracle"))]
    pub fn unchecked(&self) -> Arc<S> {
        Arc::clone(&self.src)
    }
}
