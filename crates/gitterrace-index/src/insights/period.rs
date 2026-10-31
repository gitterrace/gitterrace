use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::{Insights, bucket_label, bucket_start, fill, local_day, next_bucket};
use crate::Granularity;
use crate::aggregates::weekday;

const DAY: i64 = super::DAY;

/// Reporting window, ending at the newest commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Period {
    Week,
    Month,
    Quarter,
    Year,
    All,
}

impl Period {
    pub fn days(self) -> Option<i64> {
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
    pub fn window(self, now: i64) -> (i64, i64) {
        match self.days() {
            Some(d) => (now + 1 - d * DAY, now + 1),
            None => (i64::MIN, now + 1),
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
    pub commits: u32,
    pub additions: u64,
    pub deletions: u64,
}

/// When work happens, in each commit's own timezone.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Rhythm {
    pub weekly: Vec<SeriesPoint>,
    pub monthly: Vec<SeriesPoint>,
    /// `[weekday][hour]`, Monday is 0.
    pub punch_card: [[u32; 24]; 7],
    pub punch_max: u32,
    pub by_weekday: [u32; 7],
    pub by_hour: [u32; 24],
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
    /// Commits per week, aligned with [`ContributorReport::weeks`].
    pub weekly: Vec<u32>,
    /// First commit ever falls inside the period.
    pub first_time: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ContributorReport {
    /// Monday of every week of the period, `YYYY-MM-DD`.
    pub weeks: Vec<String>,
    /// Most commits first. Bots are included and flagged; the caller toggles them.
    pub contributors: Vec<Contributor>,
}

impl Insights<'_> {
    fn stats_between(&self, from: i64, to: i64) -> PeriodStats {
        let mut people = HashSet::new();
        let mut files: HashSet<&str> = HashSet::new();
        let mut s = PeriodStats::default();
        for (i, r) in self.records.iter().enumerate() {
            if r.time < from || r.time >= to || !self.counted(i) {
                continue;
            }
            s.commits += 1;
            people.insert(self.person[i]);
            for f in &r.files {
                if let Some(l) = self.lines(f) {
                    files.insert(f.path.as_str());
                    s.additions += u64::from(l.add);
                    s.deletions += u64::from(l.del);
                }
            }
        }
        s.authors = people.len();
        s.files = files.len();
        s
    }

    pub fn period_summary(&self, period: Period) -> PeriodSummary {
        let (from, to) = period.window(self.now);
        let current = self.stats_between(from, to);
        let previous = period
            .days()
            .map(|d| self.stats_between(from - d * DAY, from));
        PeriodSummary {
            period,
            from: period.days().map(|_| from),
            to,
            current,
            previous,
        }
    }

    pub fn rhythm(&self, period: Period) -> Rhythm {
        let (from, to) = period.window(self.now);
        let mut weeks: BTreeMap<i64, SeriesPoint> = BTreeMap::new();
        let mut months: BTreeMap<i64, SeriesPoint> = BTreeMap::new();
        let mut punch = [[0u32; 24]; 7];
        let mut by_weekday = [0u32; 7];
        let mut by_hour = [0u32; 24];
        for (i, r) in self.records.iter().enumerate() {
            if r.time < from || r.time >= to || !self.counted(i) {
                continue;
            }
            let day = local_day(r);
            let l = self.commit_lines(i);
            for (map, g) in [
                (&mut weeks, Granularity::Week),
                (&mut months, Granularity::Month),
            ] {
                let start = bucket_start(day, g);
                let p = map
                    .entry(start)
                    .or_insert_with(|| empty_point(bucket_label(start, g)));
                p.commits += 1;
                p.additions += u64::from(l.add);
                p.deletions += u64::from(l.del);
            }
            let local = r.time + i64::from(r.offset);
            let wd = weekday(day);
            let hour = usize::try_from(local.rem_euclid(DAY) / 3600).unwrap_or(0);
            punch[wd][hour] += 1;
            by_weekday[wd] += 1;
            by_hour[hour] += 1;
        }
        Rhythm {
            weekly: fill(weeks, Granularity::Week, empty_point),
            monthly: fill(months, Granularity::Month, empty_point),
            punch_max: punch.iter().flatten().copied().max().unwrap_or(0),
            punch_card: punch,
            by_weekday,
            by_hour,
        }
    }

    pub fn contributors(&self, period: Period) -> ContributorReport {
        let (from, to) = period.window(self.now);
        let first_ever: Vec<i64> = {
            let mut v = vec![i64::MAX; self.people.list.len()];
            for (r, p) in self.records.iter().zip(&self.person) {
                v[*p] = v[*p].min(r.time);
            }
            v
        };
        // Weeks of the period, from the first counted commit when the period is open ended.
        let mut in_period: Vec<usize> = (0..self.records.len())
            .filter(|i| {
                let r = &self.records[*i];
                r.time >= from && r.time < to && !r.merge
            })
            .collect();
        in_period.sort_unstable();
        let (Some(first), Some(last)) = (
            in_period.iter().map(|i| local_day(&self.records[*i])).min(),
            in_period.iter().map(|i| local_day(&self.records[*i])).max(),
        ) else {
            return ContributorReport {
                weeks: Vec::new(),
                contributors: Vec::new(),
            };
        };
        let (w0, w1) = (
            bucket_start(first, Granularity::Week),
            bucket_start(last, Granularity::Week),
        );
        let mut weeks = Vec::new();
        let mut at = w0;
        while at <= w1 {
            weeks.push(bucket_label(at, Granularity::Week));
            at = next_bucket(at, Granularity::Week);
        }
        let mut acc: std::collections::HashMap<usize, Acc> = std::collections::HashMap::new();
        for i in in_period {
            let r = &self.records[i];
            let a = acc.entry(self.person[i]).or_insert_with(|| Acc {
                c: super::Lines::default(),
                commits: 0,
                first: r.time,
                last: r.time,
                days: BTreeSet::new(),
                weekly: vec![0; weeks.len()],
            });
            let l = self.commit_lines(i);
            a.c.add += l.add;
            a.c.del += l.del;
            a.commits += 1;
            a.first = a.first.min(r.time);
            a.last = a.last.max(r.time);
            let day = local_day(r);
            a.days.insert(day);
            let w = usize::try_from((bucket_start(day, Granularity::Week) - w0) / 7).unwrap_or(0);
            a.weekly[w] += 1;
        }
        let mut contributors: Vec<(usize, Contributor)> = acc
            .into_iter()
            .map(|(p, a)| {
                let person = &self.people.list[p];
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
                        weekly: a.weekly,
                        first_time: first_ever[p] >= from && period.days().is_some(),
                    },
                )
            })
            .collect();
        contributors.sort_by(|a, b| {
            b.1.commits
                .cmp(&a.1.commits)
                .then_with(|| a.1.name.cmp(&b.1.name))
        });
        ContributorReport {
            weeks,
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
    weekly: Vec<u32>,
}

fn empty_point(key: String) -> SeriesPoint {
    SeriesPoint {
        key,
        commits: 0,
        additions: 0,
        deletions: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::Period;
    use crate::insights::fixtures::{file, rec};
    use crate::{Insights, Options};

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
        assert_eq!(r.weekly.len(), 3);
        assert_eq!(r.weekly[1].commits, 0);
        assert_eq!(r.punch_card[0][12], 1); // Monday noon
        assert_eq!(r.punch_card[2][1], 1); // day 15 is a Tuesday, 12:00 + 13h is Wednesday 01:00
        assert_eq!(r.punch_max, 1);
        assert_eq!(r.monthly.len(), 1);
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
        assert_eq!(rep.weeks.len(), 3);
        assert_eq!(rep.contributors[0].name, "Ann");
        assert_eq!(rep.contributors[0].weekly, vec![1, 0, 1]);
        assert_eq!(rep.contributors[0].additions, 6);
        assert!(rep.contributors.iter().any(|c| c.bot));
        let m = ins.contributors(Period::Week);
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
