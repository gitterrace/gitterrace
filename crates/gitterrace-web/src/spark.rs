//! The activity sparkline's window: how far back it reaches and how wide one bucket is, fitted
//! to the repository's age. Shared by the overview sidebar and the repository list, so both
//! bucket the same way.
//!
//! The rule: the span is the repository's age (now minus its first commit), capped by the
//! caller. The bucket is the finest rung of [`LADDER`] that keeps the count at or below
//! [`MAX`]. Neighbouring rungs differ by at most 7x and `MAX` is 7 x [`MIN`], so any age of
//! `MIN` hours or more lands in `MIN..=MAX` buckets (a younger one gets one bucket per hour).
//! Buckets are counted back from an end: `now` for hour rungs, the end of the local today for
//! day and week rungs, so the last day bucket is the same day "N today" counts. The last
//! bucket holds now, the first one holds the first commit, so time before the repository
//! existed never draws a flat run. A time before the window is dropped, never piled into the
//! first bucket.

use gitterrace_git::DAY;
use gitterrace_index::feed::local_day_of;

use crate::fmt;

const HOUR: i64 = 3_600;
pub(crate) const WEEK: i64 = 7 * DAY;
/// Fewest buckets a window of `MIN` hours or more gets, so the line still has a shape.
const MIN: i64 = 6;
/// Most buckets: past this the line is too dense to read in a sidebar's width. Not
/// configurable: a chart definition, not a resource limit.
const MAX: i64 = 7 * MIN;

/// One bucket width and how a label names it: "per {per}".
struct Rung {
    secs: i64,
    per: &'static str,
}

/// Finest first. A month rung is never reached: weeks hold `MAX` buckets up to ten months,
/// past both callers' caps.
const LADDER: [Rung; 4] = [
    Rung { secs: HOUR, per: "hour" },
    Rung { secs: 6 * HOUR, per: "6 hours" },
    Rung { secs: DAY, per: "day" },
    Rung { secs: WEEK, per: "week" },
];

/// Commit counts over the fitted window, oldest bucket first.
#[derive(Debug, Default)]
pub(crate) struct Series {
    pub(crate) counts: Vec<u32>,
    /// The bucket in words, as in "per day".
    pub(crate) per: &'static str,
    /// The window in words, "9 days"; empty for an empty repository.
    pub(crate) span: String,
}

impl Series {
    /// "Commits per day, last 9 days".
    pub(crate) fn label(&self) -> String {
        format!("Commits per {}, last {}", self.per, self.span)
    }
}

/// Where the window ends: `now`, and the end of the local today (local midnight after
/// `now`, in the zone the caller's "today" uses).
#[derive(Clone, Copy)]
pub(crate) struct End {
    pub(crate) now: i64,
    pub(crate) day_end: i64,
}

impl End {
    /// The end of the day holding `now` in a zone `offset` seconds east of UTC.
    pub(crate) const fn local(now: i64, offset: i32) -> Self {
        Self { now, day_end: (local_day_of(now, offset) + 1) * DAY - offset as i64 }
    }

    const fn of(self, rung: &Rung) -> i64 {
        if rung.secs >= DAY { self.day_end } else { self.now }
    }
}

/// Buckets `times` (author seconds, any order) into the window fitted to a history that began
/// at `first`, at most `cap` seconds long. `None` for `first` is an empty repository: no
/// buckets. A time before the window (a capped history, a skewed clock) is dropped; a time
/// after the end counts in the last bucket.
pub(crate) fn series(
    times: impl IntoIterator<Item = i64>,
    first: Option<i64>,
    end: End,
    cap: i64,
) -> Series {
    let Some(first) = first else {
        return Series::default();
    };
    let (rung, n) = fit(first, end, cap);
    let stop = end.of(rung);
    let mut counts = vec![0u32; usize::try_from(n).unwrap_or(1)];
    let last = counts.len() - 1;
    for t in times {
        let back = usize::try_from((stop - 1 - t).div_euclid(rung.secs).max(0)).unwrap_or(0);
        if back <= last {
            counts[last - back] += 1;
        }
    }
    Series { counts, per: rung.per, span: span_words(rung, n) }
}

/// The rung and bucket count for a history from `first` to `end`: the finest rung with at most
/// `MAX` buckets. `(stop - 1 - first) / secs + 1` buckets ending at the rung's stop reach back
/// to the bucket holding `first`; the cap bounds them.
fn fit(first: i64, end: End, cap: i64) -> (&'static Rung, i64) {
    let count = |r: &Rung| {
        let span = (end.of(r) - 1 - first).max(0);
        (span / r.secs + 1).min((cap / r.secs).max(1))
    };
    let rung = LADDER.iter().find(|r| count(r) <= MAX).unwrap_or(&LADDER[LADDER.len() - 1]);
    (rung, count(rung))
}

/// The window's length in the bucket's own unit: "9 days", "26 weeks". Six hour buckets read
/// in days, rounded up: "last 3 days" for 9 buckets, never "54 hours".
fn span_words(rung: &Rung, n: i64) -> String {
    match rung.secs {
        HOUR => fmt::count(n, "hour", "hours"),
        DAY => fmt::count(n, "day", "days"),
        WEEK => fmt::count(n, "week", "weeks"),
        s => fmt::count((n * s + DAY - 1) / DAY, "day", "days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_710_028_800;
    const CAP: i64 = 26 * WEEK;

    /// About fifty commits spread evenly from `age` ago up to now.
    fn history(age: i64) -> Vec<i64> {
        let step = (age / 50).max(60);
        (0..=age / step).map(|i| NOW - age + i * step).collect()
    }

    /// Now at 14:00 local in a zone one hour east, so day buckets end at the next local
    /// midnight, ten hours after now.
    const AT: End = End::local(NOW + 13 * HOUR, 3_600);

    /// Fits a repository `age` old: the bucket count is in range, bucket 0 holds the first
    /// commit (nothing before it), the last bucket holds now, and every commit inside the
    /// window is counted once.
    fn check(age: i64, lo: usize) -> Series {
        let now = AT.now;
        let times: Vec<i64> = history(age).iter().map(|t| t + 13 * HOUR).collect();
        let s = series(times.iter().copied(), Some(now - age), AT, CAP);
        let n = s.counts.len();
        assert!((lo..=MAX as usize).contains(&n), "age {age}: {n} buckets");
        let rung = fit(now - age, AT, CAP).0;
        let start = AT.of(rung) - i64::try_from(n).expect("n") * rung.secs;
        if age < CAP {
            assert!(start <= now - age, "age {age}: first commit before bucket 0");
            assert!(
                start + rung.secs > now - age,
                "age {age}: bucket 0 is time before the first commit"
            );
            assert!(s.counts[0] > 0, "age {age}: leading empty bucket {:?}", s.counts);
        }
        assert!(s.counts[n - 1] > 0, "age {age}: now is not in the last bucket");
        assert_eq!(
            s.counts.iter().sum::<u32>() as usize,
            times.iter().filter(|t| **t >= start).count(),
            "age {age}: every commit in the window counted once"
        );
        s
    }

    #[test]
    fn two_hours_reads_hourly() {
        let s = check(2 * HOUR, 2);
        assert_eq!(s.label(), "Commits per hour, last 2 hours");
    }

    #[test]
    fn two_days_reads_in_six_hour_buckets() {
        let s = check(2 * DAY, MIN as usize);
        assert_eq!(s.label(), "Commits per 6 hours, last 2 days");
        let s = check(2 * DAY + 7 * HOUR, MIN as usize);
        assert_eq!(s.label(), "Commits per 6 hours, last 3 days", "days round up");
    }

    #[test]
    fn three_weeks_reads_daily() {
        let s = check(3 * WEEK, MIN as usize);
        assert_eq!(s.label(), "Commits per day, last 22 days");
    }

    #[test]
    fn four_months_reads_weekly() {
        let s = check(120 * DAY, MIN as usize);
        assert_eq!(s.label(), "Commits per week, last 18 weeks");
    }

    #[test]
    fn five_years_is_capped() {
        let s = check(5 * 365 * DAY, MIN as usize);
        assert_eq!(s.label(), "Commits per week, last 26 weeks");
        let s = series(history(5 * 365 * DAY), Some(NOW - 5 * 365 * DAY), AT, 12 * WEEK);
        assert_eq!(s.label(), "Commits per week, last 12 weeks");
    }

    #[test]
    fn an_empty_repository_has_no_buckets() {
        let s = series([], None, AT, CAP);
        assert!(s.counts.is_empty() && s.span.is_empty());
    }

    /// Every age from `MIN` hours to past the cap, hour by hour, under both callers' caps:
    /// the range holds everywhere, not only at the sampled ages.
    #[test]
    fn every_age_lands_in_range() {
        for cap in [12 * WEEK, CAP] {
            for age in (MIN * HOUR..=cap + WEEK).step_by(3_600) {
                let (r, n) = fit(AT.now - age, AT, cap);
                assert!((MIN..=MAX).contains(&n), "age {age}: {n} per {}", r.per);
            }
        }
    }

    /// A time before the window is dropped, not piled into bucket 0; a future one counts last.
    #[test]
    fn a_time_outside_the_window_never_piles_into_an_end_bucket() {
        let end = End::local(NOW, 0);
        let s = series([NOW - 3 * DAY, NOW + HOUR], Some(NOW - 2 * DAY), end, CAP);
        assert_eq!((s.counts.first(), s.counts.last()), (Some(&0), Some(&1)));
        let old = series([NOW - CAP - DAY], Some(NOW - 2 * CAP), end, CAP);
        assert_eq!(old.counts.iter().sum::<u32>(), 0, "past a capped window");
    }

    /// Day buckets are local days: a commit at 23:30 local yesterday is in the bucket before
    /// the last, one at 00:30 local today in the last, the one "N today" counts.
    #[test]
    fn day_buckets_are_local_days() {
        let first = AT.now - 12 * DAY;
        let midnight = AT.day_end - DAY;
        let s = series([first, midnight - HOUR / 2, midnight + HOUR / 2], Some(first), AT, CAP);
        assert_eq!(s.per, "day");
        let n = s.counts.len();
        assert_eq!((s.counts[n - 2], s.counts[n - 1]), (1, 1), "{:?}", s.counts);
    }
}
