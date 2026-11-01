//! The one statistics layer: every number the web pages, the HTTP API and the MCP server show
//! is computed here from cached commit records, under the same [`Options`]. Everything is pure
//! over [`CommitRecord`]s (plus head-tree file lists for "now" questions), so it is
//! unit-testable and the callers only render the returned plain structs.
//!
//! Conventions: records are oldest first as [`crate::Index::records`] returns them. Commit-level
//! numbers count non-merge commits only (a merge's diff against its first parent repeats the
//! branch). Line-level series that must add up exactly (language lines over time) follow the
//! first-parent chain instead, where merge diffs are the only way a branch's lines arrive.

mod axis;
mod code;
pub(crate) mod fasthash;
mod files;
mod filter;
mod headfiles;
mod lang;
mod misc;
mod people;
mod period;
mod select;
mod stats;

use std::sync::{Arc, OnceLock};

pub(crate) use gitterrace_git::days_from_civil;
use gitterrace_git::{IgnoreSet, civil_from_days as civil};

use crate::{CommitRecord, FileStat};
use fasthash::FastMap;

pub use axis::{BucketUnit, TimeAxis};
pub use code::{CodeFrequency, Rework, ReworkPoint, rework_share, rework_verdict};
pub use files::{
    AgeBucket, AgeReport, Coupling, DirOwnership, Hotspot, HotspotReport, Owner, OwnershipReport,
    Touch,
};
pub use filter::{PathFilter, is_generated};
pub(crate) use headfiles::CACHE_FILE;
pub use headfiles::{HeadFile, LineCache, head_files};
pub use lang::language;
pub use misc::{Cadence, Comparison, LangDelta, Release, SizeBucket, release_tags};
pub use people::{People, Person};
pub use period::{
    Contributor, ContributorReport, Period, PeriodStats, PeriodSummary, Rhythm, SeriesPoint,
};
pub use select::{OptionParams, Selection, under};
pub use stats::{DirChurn, FileChurn, Label, LabelBucket, LabelCount};

pub(crate) const DAY: i64 = 86_400;

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
pub(crate) struct Lines {
    pub add: u32,
    pub del: u32,
    pub rewritten: u32,
}

/// The records plus everything derived once from them: identities, path filter, caches.
pub struct Insights<'a> {
    pub(crate) records: &'a [CommitRecord],
    pub(crate) opts: Options,
    pub(crate) filter: PathFilter,
    pub(crate) people: People,
    /// Person index per record.
    pub(crate) person: Vec<usize>,
    /// Newest commit time per person (all commits, bots included).
    pub(crate) last_seen: Vec<i64>,
    /// Newest commit time overall: "now" for periods, so old repositories still show data.
    pub(crate) now: i64,
    /// Per record, per file: whether path statistics leave the file out. Built on first use and
    /// decided once per distinct path, so no hot loop asks the filter (or its lock) again.
    skip: OnceLock<SkipTable>,
    /// Lines changed by each commit over counted files, computed on first use and shared by
    /// every statistic that sums a commit (sizes, rhythm, code frequency, contributors).
    commit_lines: OnceLock<Vec<Lines>>,
    follow: OnceLock<files::Follow<'a>>,
}

impl<'a> Insights<'a> {
    /// `ignore` is the ignore set of the head revision, see [`IgnoreSet::at`]. It is only used
    /// when `opts.respect_ignore` is set.
    pub fn new(records: &'a [CommitRecord], opts: Options, ignore: Option<IgnoreSet>) -> Self {
        let people = People::build(records);
        let person: Vec<usize> = records
            .iter()
            .map(|r| people.index_of(&r.author, &r.email))
            .collect();
        let mut last_seen = vec![i64::MIN; people.list.len()];
        for (r, p) in records.iter().zip(&person) {
            last_seen[*p] = last_seen[*p].max(r.time);
        }
        let now = records.iter().map(|r| r.time).max().unwrap_or(0);
        let filter = PathFilter::new(&opts, ignore);
        Self {
            records,
            opts,
            filter,
            people,
            person,
            last_seen,
            now,
            skip: OnceLock::new(),
            commit_lines: OnceLock::new(),
            follow: OnceLock::new(),
        }
    }

    /// True when path statistics leave `path` out.
    pub fn skips(&self, path: &str) -> bool {
        self.filter.skips(path)
    }

    /// Lines of file `fi` of record `i`, `None` when path statistics leave the file out.
    pub(crate) fn lines(&self, i: usize, fi: usize) -> Option<Lines> {
        (!self
            .skip
            .get_or_init(|| SkipTable::new(self.records, &self.filter))
            .get(i, fi))
        .then(|| self.lines_unfiltered(&self.records[i].files[fi]))
    }

    /// Every counted file of record `i` with its lines.
    pub(crate) fn files_of(&self, i: usize) -> impl Iterator<Item = (&'a FileStat, Lines)> + '_ {
        let r = &self.records[i];
        (0..r.files.len()).filter_map(move |fi| self.lines(i, fi).map(|l| (&r.files[fi], l)))
    }

    /// Line counts of a file under the whitespace option, whether or not the path filter
    /// would skip it.
    pub(crate) fn lines_unfiltered(&self, f: &FileStat) -> Lines {
        if self.opts.count_whitespace {
            Lines {
                add: f.additions,
                del: f.deletions,
                rewritten: f.rewritten,
            }
        } else {
            let c = f.code_stat();
            Lines {
                add: c.additions,
                del: c.deletions,
                rewritten: c.rewritten,
            }
        }
    }

    /// Non-merge commit by someone who counts under the options.
    pub(crate) fn counted(&self, i: usize) -> bool {
        !self.records[i].merge && (self.opts.include_bots || !self.people.list[self.person[i]].bot)
    }

    /// Lines changed by commit `i` over counted files.
    pub(crate) fn commit_lines(&self, i: usize) -> Lines {
        self.commit_lines.get_or_init(|| {
            (0..self.records.len())
                .map(|i| {
                    self.files_of(i).fold(Lines::default(), |mut out, (_, l)| {
                        out.add += l.add;
                        out.del += l.del;
                        out.rewritten += l.rewritten;
                        out
                    })
                })
                .collect()
        })[i]
    }

    /// Buckets for a time chart of `period` (`None` and `All` span the records).
    pub(crate) fn axis(&self, period: Option<Period>) -> TimeAxis {
        let days = || self.records.iter().map(local_day);
        let newest = self.records.iter().max_by_key(|r| r.time).map(local_day);
        match (days().min(), newest) {
            (Some(first), Some(last)) => TimeAxis::for_period(period, first, last),
            _ => TimeAxis::for_period(period, 1, 0),
        }
    }

    /// Bucket width of every time chart for `period`.
    pub fn bucket_unit(&self, period: Period) -> BucketUnit {
        self.axis(Some(period)).unit()
    }

    pub(crate) fn follow(&self) -> &files::Follow<'a> {
        self.follow.get_or_init(|| self.build_follow())
    }
}

/// Whether path statistics leave each file of each record out, flat: record `i`'s files start
/// at `start[i]`. Each distinct path is decided once, the decisions spread over the cores: on a
/// large history the ignore rules cost more than everything else `Insights::new` does.
struct SkipTable {
    start: Vec<usize>,
    skip: Vec<bool>,
}

impl SkipTable {
    fn new(records: &[CommitRecord], filter: &PathFilter) -> Self {
        let mut start = Vec::with_capacity(records.len() + 1);
        let mut slots: Vec<u32> = Vec::new();
        let mut slot: FastMap<usize, u32> = FastMap::default();
        let mut paths: Vec<&str> = Vec::new();
        for r in records {
            start.push(slots.len());
            for f in &r.files {
                // The loader shares one `Arc<str>` per distinct path: key by address.
                let key = Arc::as_ptr(&f.path).cast::<u8>() as usize;
                slots.push(*slot.entry(key).or_insert_with(|| {
                    paths.push(&f.path);
                    u32::try_from(paths.len() - 1).unwrap_or(u32::MAX)
                }));
            }
        }
        start.push(slots.len());
        let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let chunk = paths.len().div_ceil(threads).max(1);
        let decided: Vec<bool> = std::thread::scope(|s| {
            #[expect(
                clippy::needless_collect,
                reason = "all threads must be spawned before the first join, or they would run one after another"
            )]
            let parts: Vec<_> = paths
                .chunks(chunk)
                .map(|c| s.spawn(move || c.iter().map(|p| filter.decide(p)).collect::<Vec<_>>()))
                .collect();
            parts
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        let skip = slots
            .into_iter()
            .map(|s| decided.get(s as usize).copied().unwrap_or(false))
            .collect();
        Self { start, skip }
    }

    fn get(&self, i: usize, fi: usize) -> bool {
        self.skip[self.start[i] + fi]
    }
}

pub(crate) fn local_time(r: &CommitRecord) -> i64 {
    r.time + i64::from(r.offset)
}

pub(crate) fn local_day(r: &CommitRecord) -> i64 {
    local_time(r).div_euclid(DAY)
}

/// Monday is 0. Day 0 (1970-01-01) was a Thursday.
pub(crate) fn weekday(days: i64) -> usize {
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

pub(crate) fn year_of(r: &CommitRecord) -> i64 {
    civil(local_day(r)).0
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
mod tests {
    use super::bus_factor;

    #[test]
    fn bus_factor_counts_who_hold_at_least_half() {
        // The 50/50 split is the case where the surfaces disagreed.
        assert_eq!(bus_factor([5.0, 5.0]), 1);
        assert_eq!(bus_factor([6.0, 3.0, 1.0]), 1);
        assert_eq!(bus_factor([3.0, 3.0, 3.0, 1.0]), 2);
        assert_eq!(bus_factor([1.0, 4.0]), 1);
        assert_eq!(bus_factor([0.0, 0.0]), 0);
        assert_eq!(bus_factor(std::iter::empty()), 0);
    }
}
