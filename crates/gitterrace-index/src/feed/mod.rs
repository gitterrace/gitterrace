//! The one seam every insights page reads through: [`InsightsSource`].
//!
//! A page never knows whether its data comes from today's records in memory ([`MemorySource`]) or from the segment
//! files (`segments.rs`, `disk.rs`).
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

pub(crate) mod disk;
mod fresh;
mod memory;
mod reach;
pub(crate) mod segments;

use std::ops::Range;
use std::sync::Arc;

pub use crate::seg::{IdentId, LabelId, Oid, PathId, Rank};
#[doc(hidden)]
pub use disk::testing as disk_testing;
pub use disk::{DiskSource, IdentMap, RowWriter, block_cache_default};
pub use fresh::FRESH_CAP;
pub use memory::{IdentPairs, MemorySource};
pub use reach::{Reach, RevView, rev_view};
pub use segments::SegmentSource;

/// Where "now" comes from. Serving reads [`SystemClock`]; tests pass a [`FixedClock`], so no
/// count depends on when a test runs.
pub trait Clock: Send + Sync {
    /// Seconds since the epoch.
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

/// [`crate::reference_time`] over a source: the newest author time not after the wall clock.
pub fn reference_time(src: &dyn InsightsSource) -> i64 {
    reference_time_at(src, &SystemClock)
}

/// [`reference_time`] with the clock given.
pub fn reference_time_at(src: &dyn InsightsSource, clock: &dyn Clock) -> i64 {
    let wall = clock.now();
    src.commits_in(CommitRange::All)
        .map(|c| c.time)
        .filter(|t| *t <= wall)
        .max()
        .unwrap_or(0)
}

/// [`crate::local_time`] from the columns.
pub const fn local_time_of(time: i64, offset: i32) -> i64 {
    time.saturating_add(offset as i64)
}

/// [`crate::local_day`] from the columns.
pub const fn local_day_of(time: i64, offset: i32) -> i64 {
    time.saturating_add(offset as i64)
        .div_euclid(crate::insights::DAY)
}

/// Boxed streaming iterator borrowed from the source.
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
    /// Adds one file row.
    pub fn add_row(&mut self, f: &FileRow) {
        self.add += u64::from(f.add);
        self.del += u64::from(f.del);
        self.rewritten += u64::from(f.rewritten);
        let c = f.code();
        self.code_add += u64::from(c.0);
        self.code_del += u64::from(c.1);
        self.code_rewritten += u64::from(c.2);
    }

    pub const fn merge(&mut self, o: &Self) {
        self.add += o.add;
        self.del += o.del;
        self.rewritten += o.rewritten;
        self.code_add += o.code_add;
        self.code_del += o.code_del;
        self.code_rewritten += o.code_rewritten;
    }
}

/// One commit's columns. No strings: ask [`InsightsSource::commit_id`] and the string lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitRow {
    pub rank: Rank,
    /// Raw identity (mailmap applied to its strings, see the module contract).
    pub ident: IdentId,
    /// Author time, epoch seconds.
    pub time: i64,
    /// Author time zone offset in seconds, as [`crate::CommitRecord::offset`].
    pub offset: i32,
    pub merge: bool,
    pub kind: Option<LabelId>,
    pub scope: Option<LabelId>,
    /// Number of file rows.
    pub files: u32,
    /// Sums over every file row, no path filter applied.
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
    /// Line count after the commit.
    pub lines: u32,
    pub rewritten: u32,
    /// (add, del, rewritten) ignoring whitespace, `None` when equal to the raw counts.
    pub code: Option<(u32, u32, u32)>,
    pub binary: bool,
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
    /// Ranks `[start, end)`.
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

/// Read access to one indexed history. See the module docs for the contract.
pub trait InsightsSource: Send + Sync {
    /// Commits indexed (`commits_indexed` on the pages).
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// One commit's columns.
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

    /// The commit with this id, if indexed.
    fn find_commit(&self, id: &str) -> Option<Rank>;

    /// Author x day rows with `day` in `days`, ascending (day, ident).
    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow>;

    /// Path x author rows whose path is `prefix` or under it (`""` for all), ascending
    /// (path, ident).
    fn agg_path_author(&self, prefix: &str) -> Stream<'_, PathAuthorRow>;

    /// Posting list, ascending.
    fn postings(&self, key: Posting) -> Stream<'_, Hit>;

    /// Every rename edge, ascending rank. Follow walks them newest first.
    fn rename_edges(&self) -> Stream<'_, RenameEdge>;

    fn path(&self, id: PathId) -> Option<Arc<str>>;
    fn find_path(&self, path: &str) -> Option<PathId>;
    /// Number of path ids, so per-path bit vectors can be sized.
    fn paths(&self) -> usize;

    /// (name, email) of a raw identity, mailmap applied.
    fn ident(&self, id: IdentId) -> Option<(Arc<str>, Arc<str>)>;
    fn idents(&self) -> usize;

    /// Kind or scope name.
    fn label(&self, id: LabelId) -> Option<Arc<str>>;
    fn find_label(&self, name: &str) -> Option<LabelId>;
}
