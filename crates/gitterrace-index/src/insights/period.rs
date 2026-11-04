use std::collections::BTreeSet;

use crate::feed::{CommitRange, PathId, local_day_of};
use crate::insights::fasthash::{FastMap, FastSet};
use crate::insights::{BucketUnit, Insights, TimeAxis, reference_day, weekday};

const DAY: i64 = crate::insights::DAY;

/// Reporting window, ending at the newest commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Period {
    Week,
    Month,
    Quarter,
    Year,
    All,
}

/// What the contributors list is ordered and charted by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Metric {
    #[default]
    Commits,
    Added,
    Deleted,
    Net,
}

impl Metric {
    /// Parses `commits`, `added`, `deleted`, `net`. Anything else is the default.
    pub fn parse(s: &str) -> Self {
        match s {
            "added" => Self::Added,
            "deleted" => Self::Deleted,
            "net" => Self::Net,
            _ => Self::Commits,
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Commits => "commits",
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Net => "net",
        }
    }

    /// The person's value for ordering.
    fn of(self, commits: usize, add: u64, del: u64) -> i64 {
        let n = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        match self {
            Self::Commits => i64::try_from(commits).unwrap_or(i64::MAX),
            Self::Added => n(add),
            Self::Deleted => n(del),
            Self::Net => n(add).saturating_sub(n(del)),
        }
    }
}

impl Period {
    pub const fn days(self) -> Option<i64> {
        match self {
            Self::Week => Some(7),
            Self::Month => Some(30),
            Self::Quarter => Some(90),
            Self::Year => Some(365),
            Self::All => None,
        }
    }

    /// Parses `1w`, `1m`, `3m`, `1y`, `all`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "1w" | "week" => Self::Week,
            "1m" | "month" => Self::Month,
            "3m" | "quarter" => Self::Quarter,
            "1y" | "year" => Self::Year,
            "all" => Self::All,
            _ => return None,
        })
    }

    /// `[from, to)` in epoch seconds with `now` the newest commit. `All` starts at `i64::MIN`.
    pub const fn window(self, now: i64) -> (i64, i64) {
        match self.days() {
            Some(d) => (
                now.saturating_add(1).saturating_sub(d * DAY),
                now.saturating_add(1),
            ),
            None => (i64::MIN, now.saturating_add(1)),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct PeriodStats {
    pub commits: usize,
    pub authors: usize,
    pub files: usize,
    pub additions: u64,
    pub deletions: u64,
}

/// Pulse: the period against the one right before it of the same length.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PeriodSummary {
    pub period: Period,
    pub from: Option<i64>,
    pub to: i64,
    pub current: PeriodStats,
    /// `None` for `All`.
    pub previous: Option<PeriodStats>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SeriesPoint {
    pub key: String,
    pub label: String,
    pub commits: u32,
    pub additions: u64,
    pub deletions: u64,
}

/// When work happens, in each commit's own timezone.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Rhythm {
    /// One point per bucket of the period's axis, empty ones included.
    pub series: Vec<SeriesPoint>,
    pub unit: BucketUnit,
    /// `[weekday][hour]`, Monday is 0.
    pub punch_card: [[u32; 24]; 7],
    pub punch_max: u32,
    pub by_weekday: [u32; 7],
    pub by_hour: [u32; 24],
    /// Last local day of each bucket of `series`, `YYYY-MM-DD`: with the key, its bounds.
    #[serde(skip)]
    pub ends: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Contributor {
    pub name: String,
    pub emails: Vec<String>,
    pub bot: bool,
    pub commits: usize,
    pub additions: u64,
    pub deletions: u64,
    pub first_commit: i64,
    pub last_commit: i64,
    pub active_days: usize,
    /// Commits per bucket, aligned with [`ContributorReport::labels`].
    pub series: Vec<u32>,
    /// First commit ever falls inside the period.
    pub first_time: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ContributorReport {
    /// Axis label of every bucket of the period.
    pub labels: Vec<String>,
    pub unit: BucketUnit,
    /// Most commits first. Bots are included and flagged; the caller toggles them.
    pub contributors: Vec<Contributor>,
}

/// Whether `day` lies in the inclusive local day range, which is open when absent.
fn in_days(days: Option<(i64, i64)>, day: i64) -> bool {
    days.is_none_or(|(a, b)| (a..=b).contains(&day))
}

/// Epoch-second window holding every commit whose local day is in `a..=b`, whatever its
/// zone offset (offsets stay within a day).
const fn days_window(a: i64, b: i64) -> (i64, i64) {
    (
        a.saturating_sub(1).saturating_mul(DAY),
        b.saturating_add(2).saturating_mul(DAY),
    )
}

impl Insights<'_> {
    fn stats_where(&self, from: i64, to: i64, days: Option<(i64, i64)>) -> PeriodStats {
        let mut people: FastSet<usize> = FastSet::default();
        let mut files: FastSet<PathId> = FastSet::default();
        let mut s = PeriodStats::default();
        let src = self.source();
        let mut ranks = Vec::new();
        for c in src.commits_in(CommitRange::Time(from..to)) {
            if !self.counted_row(&c) || !in_days(days, local_day_of(c.time, c.offset)) {
                continue;
            }
            s.commits += 1;
            people.insert(self.person_of(c.ident));
            ranks.push(c.rank);
        }
        for (_, rows) in src.rows_for(ranks) {
            for f in &rows {
                let Some(l) = self.row_lines(f) else {
                    continue;
                };
                files.insert(f.path);
                s.additions += u64::from(l.add);
                s.deletions += u64::from(l.del);
            }
        }
        s.authors = people.len();
        s.files = files.len();
        s
    }

    /// Whether any counted commit falls in the period. Stops at the first one: a page that
    /// only needs "is this empty" must not sum every file row of the period.
    pub fn has_commits(&self, period: Period) -> bool {
        let days = self.day_span(period, None);
        let (from, to) = days.map_or((0, 0), |(a, b)| days_window(a, b));
        self.source()
            .commits_in(CommitRange::Time(from..to))
            .any(|c| self.counted_row(&c) && in_days(days, local_day_of(c.time, c.offset)))
    }

    /// The local days (inclusive) one page section counts: `days` when given, else the
    /// period ending on the reference day, else everything up to it. One counted set for the
    /// pulse table, the series, the weekday split and the punch card alike, so they agree
    /// whatever a commit's zone offset. `None` when there is no history.
    fn day_span(&self, period: Period, days: Option<(i64, i64)>) -> Option<(i64, i64)> {
        if days.is_some() {
            return days;
        }
        let last = reference_day(self.source())?;
        Some(period.days().map_or((i64::MIN, last), |d| {
            (last.saturating_add(1).saturating_sub(d), last)
        }))
    }

    pub fn period_summary(&self, period: Period) -> PeriodSummary {
        self.period_summary_in(period, None)
    }

    /// [`Insights::period_summary`] limited to the local days `days` (inclusive) when given:
    /// those days against the same number of days right before them.
    pub fn period_summary_in(&self, period: Period, days: Option<(i64, i64)>) -> PeriodSummary {
        let span = self.day_span(period, days);
        let count = |r: Option<(i64, i64)>| {
            r.map_or_else(PeriodStats::default, |(a, b)| {
                let (lo, hi) = days_window(a, b);
                self.stats_where(lo, hi, Some((a, b)))
            })
        };
        let len = days.map(|(a, b)| b - a + 1).or_else(|| period.days());
        let previous = len.and_then(|n| span.map(|(a, _)| count(Some((a - n, a - 1)))));
        let (from, to) = days.map_or_else(
            || {
                let (f, t) = period.window(self.now);
                (period.days().map(|_| f), t)
            },
            |(a, b)| (Some(a * DAY), (b + 1) * DAY),
        );
        PeriodSummary {
            period,
            from,
            to,
            current: count(span),
            previous: if len.is_some() {
                previous.or_else(|| Some(PeriodStats::default()))
            } else {
                None
            },
        }
    }

    pub fn rhythm(&self, period: Period) -> Rhythm {
        self.rhythm_in(period, None)
    }

    /// [`Insights::rhythm`] limited to the local days `days` (inclusive, days since the
    /// epoch): the series, weekday, hour and punch card counts all cover only that range,
    /// and the series buckets span just the range.
    pub fn rhythm_in(&self, period: Period, days: Option<(i64, i64)>) -> Rhythm {
        let span = self.day_span(period, days);
        let (from, to) = span.map_or((0, 0), |(a, b)| days_window(a, b));
        let axis = match days {
            Some((a, b)) => TimeAxis::for_period(None, a, b),
            None => self.axis(Some(period)),
        };
        let mut series: Vec<SeriesPoint> = axis
            .names()
            .map(|(key, label)| SeriesPoint {
                key,
                label,
                commits: 0,
                additions: 0,
                deletions: 0,
            })
            .collect();
        let mut punch = [[0u32; 24]; 7];
        let mut by_weekday = [0u32; 7];
        let mut by_hour = [0u32; 24];
        for c in self.source().commits_in(CommitRange::Time(from..to)) {
            if !self.counted_row(&c) {
                continue;
            }
            let day = local_day_of(c.time, c.offset);
            if !in_days(span, day) {
                continue;
            }
            let l = self.commit_sums(c.rank);
            if let Some(p) = axis.slot_day(day).map(|s| &mut series[s]) {
                p.commits += 1;
                p.additions += u64::from(l.add);
                p.deletions += u64::from(l.del);
            }
            let local = c.time.saturating_add(i64::from(c.offset));
            let wd = weekday(day);
            let hour = usize::try_from(local.rem_euclid(DAY) / 3600).unwrap_or(0);
            punch[wd][hour] += 1;
            by_weekday[wd] += 1;
            by_hour[hour] += 1;
        }
        Rhythm {
            series,
            unit: axis.unit(),
            punch_max: punch.iter().flatten().copied().max().unwrap_or(0),
            punch_card: punch,
            by_weekday,
            by_hour,
            ends: (0..axis.len()).map(|i| axis.end_key(i)).collect(),
        }
    }

    pub fn contributors(&self, period: Period) -> ContributorReport {
        self.contributors_by(period, Metric::Commits)
    }

    /// Contributors ordered by `metric`. The series carries that metric per bucket (net is
    /// floored at zero), the other fields are the same.
    pub fn contributors_by(&self, period: Period, metric: Metric) -> ContributorReport {
        let (from, to) = period.window(self.now);
        let src = self.source();
        // First commit ever per person, merges included, over the whole history.
        let mut first_ever: FastMap<usize, i64> = FastMap::default();
        for c in src.commits_in(CommitRange::All) {
            let t = first_ever
                .entry(self.person_of(c.ident))
                .or_insert(i64::MAX);
            *t = (*t).min(c.time);
        }
        let axis = self.axis(Some(period));
        let labels: Vec<String> = (0..axis.len()).map(|i| axis.label(i)).collect();
        let mut acc: std::collections::HashMap<usize, Acc> = std::collections::HashMap::new();
        for c in src.commits_in(CommitRange::Time(from..to)) {
            if c.merge {
                continue;
            }
            let a = acc.entry(self.person_of(c.ident)).or_insert_with(|| Acc {
                c: super::Lines::default(),
                commits: 0,
                first: c.time,
                last: c.time,
                days: BTreeSet::new(),
                series: vec![0; axis.len()],
                dels: vec![0; axis.len()],
            });
            let l = self.commit_sums(c.rank);
            a.c.add += l.add;
            a.c.del += l.del;
            a.commits += 1;
            a.first = a.first.min(c.time);
            a.last = a.last.max(c.time);
            let day = local_day_of(c.time, c.offset);
            a.days.insert(day);
            if let Some(slot) = axis.slot_day(day) {
                match metric {
                    Metric::Commits => a.series[slot] += 1,
                    Metric::Added | Metric::Net => {
                        a.series[slot] = a.series[slot].saturating_add(l.add);
                        a.dels[slot] = a.dels[slot].saturating_add(l.del);
                    }
                    Metric::Deleted => a.series[slot] = a.series[slot].saturating_add(l.del),
                }
            }
        }
        let mut contributors: Vec<(usize, Contributor)> = acc
            .into_iter()
            .map(|(p, a)| {
                let person = &self.d.people.list[p];
                (
                    p,
                    Contributor {
                        name: person.name.clone(),
                        emails: person.emails.clone(),
                        bot: person.bot,
                        commits: a.commits,
                        additions: u64::from(a.c.add),
                        deletions: u64::from(a.c.del),
                        first_commit: a.first,
                        last_commit: a.last,
                        active_days: a.days.len(),
                        series: if metric == Metric::Net {
                            a.series
                                .iter()
                                .zip(&a.dels)
                                .map(|(x, y)| x.saturating_sub(*y))
                                .collect()
                        } else {
                            a.series
                        },
                        first_time: first_ever.get(&p).copied().unwrap_or(i64::MAX) >= from
                            && period.days().is_some(),
                    },
                )
            })
            .collect();
        let key = |c: &Contributor| metric.of(c.commits, c.additions, c.deletions);
        contributors.sort_by(|a, b| {
            key(&b.1)
                .cmp(&key(&a.1))
                .then_with(|| a.1.name.cmp(&b.1.name))
        });
        ContributorReport {
            labels,
            unit: axis.unit(),
            contributors: contributors.into_iter().map(|(_, c)| c).collect(),
        }
    }
}

struct Acc {
    c: super::Lines,
    commits: usize,
    first: i64,
    last: i64,
    days: BTreeSet<i64>,
    series: Vec<u32>,
    /// Deleted lines per bucket, read for the net metric.
    dels: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::Period;
    use crate::insights::fixtures::{file, rec};
    use crate::{Insights, Options};

    /// Commits near UTC midnight and around every window edge, in several zone offsets.
    fn edge_records() -> Vec<crate::CommitRecord> {
        let offsets = [-12 * 3600, -5 * 3600, 0, 3600, 5 * 3600 + 1800, 14 * 3600];
        let mut out = Vec::new();
        for day in 0..400i64 {
            for (k, off) in offsets.iter().enumerate() {
                if (day + k as i64) % 3 != 0 {
                    continue;
                }
                let mut r = rec(&format!("c{day}-{k}"), "Ann", day, vec![]);
                // 23:30, 00:30 or noon UTC.
                let second = [23 * 3600 + 1800, 1800, 12 * 3600][(day as usize + k) % 3];
                r.time = r.time - 12 * 3600 + second;
                r.offset = *off;
                out.push(r);
            }
        }
        out
    }

    /// Series, weekday, hour, punch card and pulse table count one set of commits.
    #[test]
    fn every_section_counts_one_set_of_commits() {
        let recs = edge_records();
        let ins = Insights::new(&recs, Options::default(), None);
        let day0 = crate::insights::days_from_civil(2024, 1, 1);
        let ranges = [
            None,
            Some((day0 + 100, day0 + 106)),
            Some((day0 + 200, day0 + 200)),
            Some((day0 + 390, day0 + 420)),
        ];
        for period in [
            Period::Week,
            Period::Month,
            Period::Quarter,
            Period::Year,
            Period::All,
        ] {
            for days in ranges {
                let r = ins.rhythm_in(period, days);
                let series: u32 = r.series.iter().map(|p| p.commits).sum();
                let weekday: u32 = r.by_weekday.iter().sum();
                let hour: u32 = r.by_hour.iter().sum();
                let punch: u32 = r.punch_card.iter().flatten().sum();
                let kpi = ins.period_summary_in(period, days).current.commits as u32;
                let at = format!("{period:?} {days:?}");
                assert_eq!(series, weekday, "series vs weekday, {at}");
                assert_eq!(series, hour, "series vs hour, {at}");
                assert_eq!(series, punch, "series vs punch, {at}");
                assert_eq!(series, kpi, "series vs table, {at}");
                assert_eq!(
                    ins.has_commits(period),
                    days.is_some() || series > 0,
                    "{at}"
                );
            }
        }
    }

    #[test]
    fn pulse_compares_with_previous_window() {
        // Day 0 = Monday 2024-01-01. Newest commit on day 20.
        let recs = vec![
            rec("a", "Ann", 0, vec![file("x.rs", 5, 1, 4)]),
            rec(
                "b",
                "Bob",
                10,
                vec![file("x.rs", 3, 0, 7), file("Cargo.lock", 99, 0, 99)],
            ),
            rec("c", "Ann", 20, vec![file("y.rs", 2, 2, 2)]),
        ];
        let ins = Insights::new(&recs, Options::default(), None);
        let s = ins.period_summary(Period::Month);
        assert_eq!(
            (s.current.commits, s.current.authors, s.current.files),
            (3, 2, 2)
        );
        // Lock files are generated and left out.
        assert_eq!((s.current.additions, s.current.deletions), (10, 3));
        assert_eq!(s.previous.as_ref().map(|p| p.commits), Some(0));
        let w = ins.period_summary(Period::Week);
        assert_eq!(w.current.commits, 1);
        // The 7 days before the newest week include the commit on day 10.
        assert_eq!(w.previous.map(|p| p.commits), Some(1));
        assert!(ins.period_summary(Period::All).previous.is_none());
    }

    #[test]
    fn rhythm_is_dense_and_local() {
        let mut recs = vec![rec("a", "Ann", 0, vec![]), rec("b", "Ann", 15, vec![])];
        recs[1].offset = 3600 * 13; // 12:00 UTC is 01:00 the next day locally
        let r = Insights::new(&recs, Options::default(), None).rhythm(Period::All);
        // 16 local days from Mon 2024-01-01 to Tue 2024-01-16, per day.
        assert_eq!(r.series.len(), 17);
        assert_eq!(r.series[1].commits, 0);
        assert_eq!(r.series[16].commits, 1);
        assert_eq!(r.punch_card[0][12], 1); // Monday noon
        assert_eq!(r.punch_card[2][1], 1); // day 15 is a Tuesday, 12:00 + 13h is Wednesday 01:00
        assert_eq!(r.punch_max, 1);
    }

    #[test]
    fn rhythm_in_limits_weekdays_and_keeps_the_series_and_bucket_ends() {
        let recs = vec![rec("a", "Ann", 0, vec![]), rec("b", "Ann", 3, vec![])];
        let ins = Insights::new(&recs, Options::default(), None);
        let all = ins.rhythm(Period::All);
        let day0 = crate::insights::days_from_civil(2024, 1, 1);
        let one = ins.rhythm_in(Period::All, Some((day0, day0)));
        assert_eq!(one.by_weekday.iter().sum::<u32>(), 1);
        assert_eq!(one.series.iter().map(|p| p.commits).sum::<u32>(), 1);
        assert_eq!(one.series.len(), 1);
        assert_eq!(all.ends.len(), all.series.len());
        assert_eq!(all.ends[0], all.series[0].key);
    }

    #[test]
    fn contributors_have_series_and_flags() {
        let recs = vec![
            rec("a", "Ann", 0, vec![file("x.rs", 5, 1, 4)]),
            rec("b", "dependabot[bot]", 8, vec![]),
            rec("c", "Ann", 16, vec![file("x.rs", 1, 0, 5)]),
            rec("d", "Cy", 16, vec![]),
        ];
        let ins = Insights::new(&recs, Options::default(), None);
        let rep = ins.contributors(Period::All);
        assert_eq!(rep.labels.len(), rep.contributors[0].series.len());
        assert_eq!(rep.contributors[0].name, "Ann");
        assert_eq!(rep.contributors[0].series.iter().sum::<u32>(), 2);
        assert_eq!(rep.contributors[0].series[0], 1);
        assert_eq!(rep.contributors[0].additions, 6);
        assert!(rep.contributors.iter().any(|c| c.bot));
        let m = ins.contributors(Period::Week);
        assert_eq!(m.labels.len(), 7);
        assert!(
            m.contributors
                .iter()
                .all(|c| c.first_time || c.name == "Ann")
        );
        assert!(
            m.contributors
                .iter()
                .any(|c| c.name == "Cy" && c.first_time)
        );
    }
}
