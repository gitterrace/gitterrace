//! Repository insights computed from cached commit records: pulse, contributors, rhythm, code
//! frequency, languages, hotspots, ownership and more. Everything here is pure over
//! [`CommitRecord`]s (plus head-tree file lists for "now" questions), so it is unit-testable and
//! the web layer only renders the returned plain structs.
//!
//! Conventions: records are oldest first as [`crate::Index::records`] returns them. Commit-level
//! numbers count non-merge commits only (a merge's diff against its first parent repeats the
//! branch). Line-level series that must add up exactly (language lines over time) follow the
//! first-parent chain instead, where merge diffs are the only way a branch's lines arrive.

mod axis;
mod code;
mod files;
mod filter;
mod headfiles;
mod lang;
mod misc;
mod people;
mod period;

use std::sync::OnceLock;

use gitterrace_git::IgnoreSet;

use crate::aggregates::civil;
use crate::{CommitRecord, FileStat};

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

const DAY: i64 = crate::aggregates::DAY;

/// Which paths and commits count.
#[allow(clippy::struct_excessive_bools)] // independent switches, one per option
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
    follow: OnceLock<files::Follow>,
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
            follow: OnceLock::new(),
        }
    }

    /// True when path statistics leave `path` out.
    pub fn skips(&self, path: &str) -> bool {
        self.filter.skips(path)
    }

    pub(crate) fn lines(&self, f: &FileStat) -> Option<Lines> {
        (!self.filter.skips(&f.path)).then(|| self.lines_unfiltered(f))
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
        let mut out = Lines::default();
        for l in self.records[i].files.iter().filter_map(|f| self.lines(f)) {
            out.add += l.add;
            out.del += l.del;
            out.rewritten += l.rewritten;
        }
        out
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

    pub(crate) fn follow(&self) -> &files::Follow {
        self.follow.get_or_init(|| self.build_follow())
    }
}

pub(crate) fn local_day(r: &CommitRecord) -> i64 {
    (r.time + i64::from(r.offset)).div_euclid(DAY)
}

pub(crate) fn year_of(r: &CommitRecord) -> i64 {
    civil(local_day(r)).0
}

/// Inverse of `civil`.
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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
    use super::days_from_civil;
    use crate::aggregates::civil;

    #[test]
    fn civil_round_trip() {
        for d in [-1000, 0, 19_723, 19_782, 20_000] {
            let (y, m, dd) = civil(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
    }
}
