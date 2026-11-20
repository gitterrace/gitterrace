//! The one place that decides how a time chart is bucketed: the unit follows the selected
//! period, and the bucket list is complete from the window start to its end, so quiet stretches
//! show as zero instead of vanishing.

use gitterrace_git::civil_from_days as civil;

#[cfg(test)]
use crate::feed::{CommitRange, CommitRow, InsightsSource, local_day_of};
#[cfg(test)]
use crate::insights::reference_day;
use crate::insights::{Period, days_from_civil, format_date, weekday};

/// Not configurable, these four: they define chart resolution, so a setting would move every
/// golden and make the same history read differently between machines.
/// Longest `All` span, in days, that still reads best as one bar per day (60 bars).
const ALL_DAYS_MAX: i64 = 60;
/// Longest `All` span shown per week (60 bars).
const ALL_WEEKS_MAX: i64 = 420;
/// Longest `All` span shown per month (60 bars). Longer histories fall to quarters, then years.
const ALL_MONTHS_MAX: i64 = 1_800;
/// Longest `All` span shown per quarter (60 bars).
const ALL_QUARTERS_MAX: i64 = 5_460;
/// Most buckets any axis holds, whatever the dates say. Past it the oldest buckets are dropped:
/// the newest stretch is the one a reader looks for.
///
/// Not configurable: resolution comes from the span thresholds above, so a real history (under
/// 400 years) never reaches this. It only bounds the walk against corrupt or hostile dates, and a
/// lower value would drop history from the totals.
pub const MAX_BUCKETS: usize = 400;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Width of one bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum BucketUnit {
    Day,
    /// Starts on Monday.
    Week,
    Month,
    Quarter,
    Year,
}

impl BucketUnit {
    /// For hints and titles: "per day", "per week".
    pub const fn noun(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Quarter => "quarter",
            Self::Year => "year",
        }
    }

    /// First day of the bucket holding `day`.
    fn start(self, day: i64) -> i64 {
        let (y, m, _) = civil(day);
        match self {
            Self::Day => day,
            Self::Week => day - weekday(day) as i64,
            Self::Month => days_from_civil(y, m, 1),
            Self::Quarter => days_from_civil(y, (m - 1) / 3 * 3 + 1, 1),
            Self::Year => days_from_civil(y, 1, 1),
        }
    }

    /// Start of the bucket after the one starting at `start`.
    fn next(self, start: i64) -> i64 {
        let months = |n: i64| {
            let (y, m, _) = civil(start);
            let t = y * 12 + (m - 1) + n;
            days_from_civil(t.div_euclid(12), t.rem_euclid(12) + 1, 1)
        };
        match self {
            Self::Day => start + 1,
            Self::Week => start + 7,
            Self::Month => months(1),
            Self::Quarter => months(3),
            Self::Year => months(12),
        }
    }

    fn label(self, start: i64) -> String {
        let (y, m, d) = civil(start);
        let month = MONTHS[usize::try_from(m - 1).unwrap_or(0)];
        match self {
            Self::Day => format!("{} {d}", WEEKDAYS[weekday(start)]),
            Self::Week => format!("{d} {month}"),
            Self::Month => format!("{month} {y}"),
            Self::Quarter => format!("Q{} {y}", (m - 1) / 3 + 1),
            Self::Year => y.to_string(),
        }
    }
}

/// Ordered buckets covering a window completely. Days are local day numbers (days since the
/// epoch in each commit's own offset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeAxis {
    unit: BucketUnit,
    starts: Vec<i64>,
}

impl TimeAxis {
    const fn empty() -> Self {
        Self { unit: BucketUnit::Day, starts: Vec::new() }
    }

    /// The axis for `period` ending on local day `last`. A fixed period starts
    /// `period.days()` before `last`; `All` (or `None`) starts at `first`.
    ///
    /// Week and month read best per day, a quarter per week (13 bars), a year per week (52
    /// bars: a year per month would be 12 coarse bars, and 52 still fit 960px). `All` picks the
    /// unit by span so a chart holds roughly 12 to 60 bars.
    pub fn for_period(period: Option<Period>, first: i64, last: i64) -> Self {
        let first = period
            .and_then(Period::days)
            .map_or(first, |d| last.saturating_add(1).saturating_sub(d));
        if last < first {
            return Self::empty();
        }
        let span = last.saturating_sub(first).saturating_add(1);
        let unit = match period {
            Some(Period::Week | Period::Month) => BucketUnit::Day,
            Some(Period::Quarter | Period::Year) => BucketUnit::Week,
            Some(Period::All) | None => [
                (ALL_DAYS_MAX, BucketUnit::Day),
                (ALL_WEEKS_MAX, BucketUnit::Week),
                (ALL_MONTHS_MAX, BucketUnit::Month),
                (ALL_QUARTERS_MAX, BucketUnit::Quarter),
            ]
            .into_iter()
            .find(|(max, _)| span <= *max)
            .map_or(BucketUnit::Year, |(_, unit)| unit),
        };
        // Bound the walk before it starts: no record date may size the loop.
        let widest: i64 = match unit {
            BucketUnit::Day => 1,
            BucketUnit::Week => 7,
            BucketUnit::Month => 31,
            BucketUnit::Quarter => 92,
            BucketUnit::Year => 366,
        };
        let room = i64::try_from(MAX_BUCKETS).map_or(i64::MAX, |n| n.saturating_add(1));
        let floor = last.saturating_sub(widest.saturating_mul(room));
        let mut starts = Vec::new();
        let mut at = unit.start(first.max(floor));
        while at <= last {
            starts.push(at);
            at = unit.next(at);
        }
        if starts.len() > MAX_BUCKETS {
            starts.drain(..starts.len() - MAX_BUCKETS);
        }
        Self { unit, starts }
    }

    /// The axis for the commits on screen. A fixed period ends on the local day of the newest
    /// commit of `all` (the unscoped history); `All` spans `shown`. The parity oracle for the
    /// scope axis, so it lives with the tests.
    #[cfg(test)]
    pub(crate) fn covering_source(
        period: Option<Period>,
        all: &dyn InsightsSource,
        shown: &dyn InsightsSource,
    ) -> Self {
        if period.is_some_and(|p| p != Period::All) {
            return reference_day(all)
                .map_or_else(Self::empty, |last| Self::for_period(period, last, last));
        }
        let (mut lo, mut hi) = (None::<i64>, None::<i64>);
        for c in shown.commits_in(CommitRange::All) {
            let d = local_day_of(c.time, c.offset);
            lo = Some(lo.map_or(d, |x| x.min(d)));
            hi = Some(hi.map_or(d, |x| x.max(d)));
        }
        match (lo, hi) {
            (Some(first), Some(last)) => Self::for_period(period, first, last),
            _ => Self::empty(),
        }
    }

    /// Bucket of a commit, see [`TimeAxis::slot_day`].
    #[cfg(test)]
    pub(crate) fn slot_row(&self, c: &CommitRow) -> Option<usize> {
        self.slot_day(local_day_of(c.time, c.offset))
    }

    pub const fn unit(&self) -> BucketUnit {
        self.unit
    }

    pub const fn len(&self) -> usize {
        self.starts.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// Bucket holding local day `day`. A day one outside the axis still lands in the edge
    /// bucket: the window is cut in UTC, so a commit at its edge can sit a day off in its own
    /// offset. Anything further out is `None`.
    pub fn slot_day(&self, day: i64) -> Option<usize> {
        let (first, last) = (*self.starts.first()?, *self.starts.last()?);
        let end = self.unit.next(last);
        (first - 1..=end)
            .contains(&day)
            .then(|| self.starts.partition_point(|s| *s <= day).saturating_sub(1))
    }

    /// Sortable key of bucket `i`: the date it starts on.
    pub fn key(&self, i: usize) -> String {
        format_date(self.starts[i])
    }

    /// Date of the last day of bucket `i`, `YYYY-MM-DD`.
    pub fn end_key(&self, i: usize) -> String {
        format_date(self.unit.next(self.starts[i]) - 1)
    }

    /// Axis label of bucket `i`: "Mon 27", "27 Oct", "Oct 2026", "Q3 2026" or "2026".
    pub fn label(&self, i: usize) -> String {
        self.unit.label(self.starts[i])
    }

    /// Key and label of every bucket.
    pub fn names(&self) -> impl Iterator<Item = (String, String)> + '_ {
        (0..self.len()).map(|i| (self.key(i), self.label(i)))
    }
}

#[cfg(test)]
mod tests {
    use super::{BucketUnit, TimeAxis};
    use crate::Period;
    use crate::insights::days_from_civil;

    fn axis(period: Option<Period>, first: i64, last: i64) -> TimeAxis {
        TimeAxis::for_period(period, first, last)
    }

    #[test]
    fn fixed_periods_have_one_bucket_per_unit_including_empty_ones() {
        // Sunday 2024-03-31, so the weeks line up.
        let last = days_from_civil(2024, 3, 31);
        let w = axis(Some(Period::Week), last, last);
        assert_eq!((w.unit(), w.len()), (BucketUnit::Day, 7));
        assert_eq!(w.label(0), "Mon 25");
        assert_eq!(w.label(6), "Sun 31");
        let m = axis(Some(Period::Month), last, last);
        assert_eq!((m.unit(), m.len()), (BucketUnit::Day, 30));
        let q = axis(Some(Period::Quarter), last, last);
        assert_eq!((q.unit(), q.len()), (BucketUnit::Week, 13));
        assert_eq!(q.label(0), "1 Jan");
        let y = axis(Some(Period::Year), last, last);
        assert_eq!(y.unit(), BucketUnit::Week);
        assert!((52..=53).contains(&y.len()));
        // The last bucket always holds `last`, the first holds the window start.
        assert_eq!(y.slot_day(last), Some(y.len() - 1));
        assert_eq!(y.slot_day(last - 364), Some(0));
    }

    #[test]
    fn all_picks_the_unit_by_span() {
        let day = days_from_civil(2020, 1, 1);
        let unit = |span: i64| axis(None, day, day + span - 1);
        assert_eq!((unit(3).unit(), unit(3).len()), (BucketUnit::Day, 3));
        assert_eq!(unit(200).unit(), BucketUnit::Week);
        assert_eq!(unit(1000).unit(), BucketUnit::Month);
        assert_eq!(unit(4000).unit(), BucketUnit::Quarter);
        assert_eq!(unit(9000).unit(), BucketUnit::Year);
        for span in [90, 400, 1500, 5000, 20_000] {
            let n = unit(span).len();
            assert!((12..=61).contains(&n), "{span} days gave {n} buckets");
        }
    }

    #[test]
    fn labels_follow_the_unit() {
        let first = days_from_civil(2025, 11, 20);
        let q = axis(None, first, days_from_civil(2030, 6, 1));
        assert_eq!(q.unit(), BucketUnit::Month);
        assert_eq!(q.label(0), "Nov 2025");
        let years = axis(None, first, days_from_civil(2045, 6, 1));
        assert_eq!(years.unit(), BucketUnit::Year);
        assert_eq!(years.label(0), "2025");
        let quarters = axis(None, first, days_from_civil(2036, 6, 1));
        assert_eq!(quarters.unit(), BucketUnit::Quarter);
        assert_eq!(quarters.label(0), "Q4 2025");
        assert_eq!(quarters.key(0), "2025-10-01");
    }

    #[test]
    fn days_outside_clamp_and_an_empty_axis_has_no_slot() {
        let last = days_from_civil(2024, 3, 31);
        let w = axis(Some(Period::Week), last, last);
        assert_eq!(w.slot_day(last + 1), Some(6));
        assert_eq!(w.slot_day(last + 3), None);
        assert_eq!(w.slot_day(last - 8), None);
        assert_eq!(axis(None, 10, 5).slot_day(7), None);
    }

    /// The record axis the source axis must equal: a fixed period ends on the newest record's
    /// local day, `All` spans `shown`.
    fn covering(
        period: Option<Period>,
        all: &[crate::CommitRecord],
        shown: &[crate::CommitRecord],
    ) -> TimeAxis {
        use crate::insights::{local_day, reference_record};
        if period.is_some_and(|p| p != Period::All) {
            return reference_record(all).map_or_else(TimeAxis::empty, |r| {
                let last = local_day(r);
                TimeAxis::for_period(period, last, last)
            });
        }
        let days = || shown.iter().map(local_day);
        match (days().min(), days().max()) {
            (Some(first), Some(last)) => TimeAxis::for_period(period, first, last),
            _ => TimeAxis::empty(),
        }
    }

    /// The source axis equals the record axis on every period, with ties in time, a commit
    /// after the wall clock and offsets that move the local day.
    #[test]
    fn source_axis_equals_record_axis() {
        use crate::feed::{InsightsSource, MemorySource};
        use crate::insights::fixtures::{file, rec};
        use crate::insights::{local_day, reference_day, reference_record};
        let mut recs = vec![
            rec("a", "ann", 0, vec![file("x", 1, 0, 1)]),
            rec("b", "bob", 400, vec![file("x", 1, 0, 1)]),
            rec("c", "cy", 400, vec![file("y", 1, 0, 1)]),
            rec("d", "di", 900_000, vec![file("y", 1, 0, 1)]),
        ];
        recs[1].offset = -13 * 3600;
        recs[2].offset = 14 * 3600;
        let shown = &recs[1..3];
        let (all_src, shown_src) = (MemorySource::new(&recs), MemorySource::new(shown));
        assert_eq!(
            reference_day(&all_src),
            reference_record(&recs).map(local_day),
            "reference day"
        );
        for p in [None, Some(Period::Week), Some(Period::Year), Some(Period::All)] {
            let want = covering(p, &recs, shown);
            let got = TimeAxis::covering_source(p, &all_src, &shown_src);
            assert_eq!(got.names().collect::<Vec<_>>(), want.names().collect::<Vec<_>>());
            assert_eq!(got.unit(), want.unit());
        }
        let c = all_src.commit(crate::feed::Rank(2));
        assert_eq!(
            c.and_then(|c| want_slot(&recs, &c)),
            covering(None, &recs, &recs).slot_day(local_day(&recs[2]))
        );
    }

    fn want_slot(recs: &[crate::CommitRecord], c: &crate::feed::CommitRow) -> Option<usize> {
        covering(None, recs, recs).slot_row(c)
    }

    /// Every day of a history up to 399 years lands in a bucket, at every unit the span picks:
    /// the cap never drops a commit from a real history, so totals stay the same at every
    /// bucket count.
    #[test]
    fn the_bucket_cap_never_drops_a_real_day() {
        let first = days_from_civil(1700, 1, 1);
        for years in [0_i64, 1, 5, 40, 150, 399] {
            let last = first + years * 365 + 30;
            let a = axis(None, first, last);
            assert!(a.len() <= super::MAX_BUCKETS);
            let mut per = vec![0usize; a.len()];
            for d in first..=last {
                per[a.slot_day(d).unwrap_or_else(|| panic!("{years}y: day {d} dropped"))] += 1;
            }
            let total: i64 = per.iter().map(|n| i64::try_from(*n).unwrap_or(0)).sum();
            assert_eq!(total, last - first + 1, "{years}y: {:?}", a.unit());
        }
    }
}
