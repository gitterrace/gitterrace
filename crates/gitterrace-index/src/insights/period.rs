use crate::feed::SystemClock;
use crate::insights::{
    BucketUnit, CountContext, Counted, Insights, OptionParams, Scope, ScopeParams, Series, Totals,
    When, format_date,
};

mod people;

pub use gitterrace_types::Period;

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
    /// First commit ever (merges and the whole history included) is on or after the first
    /// counted local day. Never for `All`.
    pub first_time: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ContributorReport {
    /// Axis label of every bucket of the period.
    pub labels: Vec<String>,
    pub unit: BucketUnit,
    /// Most commits first. Bots count as the options say, like in every other count.
    pub contributors: Vec<Contributor>,
}

/// A scope of `period` (or, with `days`, of those local days whatever the period) over the
/// same options and selection as `base`. The "before" column of a day range and the old entry
/// points build their scope here.
fn scope_with(base: &Scope, period: Period, days: Option<(i64, i64)>) -> Scope {
    let p = ScopeParams {
        period: Some(
            if days.is_some() { Period::All } else { period }
                .as_str()
                .to_owned(),
        ),
        from: days.map(|(a, _)| format_date(a)),
        to: days.map(|(_, b)| format_date(b)),
        select: base.selection().clone(),
        options: OptionParams::default(),
    };
    Scope::from_params(&p, base.options().clone(), period).unwrap_or_else(|_| base.clone())
}

impl Insights<'_> {
    /// A scope of `period` (and `days`) over this history and its options, no selection.
    fn plain_scope(&self, period: Period, days: Option<(i64, i64)>) -> Scope {
        let base = Scope::from_params(&ScopeParams::default(), self.opts.clone(), Period::All)
            .unwrap_or_else(|_| unreachable!("an empty query is always a valid scope"));
        scope_with(&base, period, days)
    }

    /// The context over this history read from the system clock.
    fn own_context(&self) -> CountContext {
        CountContext::new(self.source(), &SystemClock)
    }

    /// Whether any counted commit falls in the period. Stops at the first one. The
    /// period tests check it against the summed counts.
    #[cfg(test)]
    pub(crate) fn has_commits(&self, period: Period) -> bool {
        let s = self.plain_scope(period, None);
        self.counted(&s, self.own_context())
            .stream()
            .next()
            .is_some()
    }

    pub fn period_summary(&self, period: Period) -> PeriodSummary {
        self.period_summary_in(period, None)
    }

    /// [`Insights::period_summary`] limited to the local days `days` (inclusive) when given:
    /// those days against the same number of days right before them.
    pub fn period_summary_in(&self, period: Period, days: Option<(i64, i64)>) -> PeriodSummary {
        let s = self.plain_scope(period, days);
        self.counted(&s, self.own_context()).summary_as(period)
    }

    pub fn rhythm(&self, period: Period) -> Rhythm {
        self.rhythm_in(period, None)
    }

    /// [`Insights::rhythm`] limited to the local days `days` (inclusive, days since the
    /// epoch): the series, weekday, hour and punch card counts all cover only that range,
    /// and the series buckets span just the range.
    pub fn rhythm_in(&self, period: Period, days: Option<(i64, i64)>) -> Rhythm {
        let s = self.plain_scope(period, days);
        self.counted(&s, self.own_context()).rhythm()
    }

    pub fn contributors(&self, period: Period) -> ContributorReport {
        self.contributors_by(period, Metric::Commits)
    }

    /// Contributors ordered by `metric`. The series carries that metric per bucket (net is
    /// floored at zero), the other fields are the same.
    pub fn contributors_by(&self, period: Period, metric: Metric) -> ContributorReport {
        let s = self.plain_scope(period, None);
        self.counted(&s, self.own_context()).contributors(metric)
    }
}

/// Epoch-second bounds `[from, to)` of the counted days, `from` `None` for all history (then
/// `to` is just after the context's "now").
const fn bounds(c: &Counted<'_>) -> (Option<i64>, i64) {
    let d = c.days();
    let day = crate::insights::DAY;
    if *d.start() == i64::MIN {
        return (None, c.context().now().saturating_add(1));
    }
    (
        Some(d.start().saturating_mul(day)),
        d.end().saturating_add(1).saturating_mul(day),
    )
}

impl Counted<'_> {
    /// Totals against the same number of days right before them: a day range against the
    /// days before it, a fixed period against the period before, `All` against nothing.
    pub fn summary(&self) -> PeriodSummary {
        self.summary_as(self.scope().period())
    }

    /// Series, weekday, hour and punch card, in one pass.
    pub fn rhythm(&self) -> Rhythm {
        let mut series = Series::new(self.axis());
        let mut when = When::default();
        self.reduce(&mut [&mut series, &mut when]);
        self.rhythm_of(series, &when)
    }

    /// [`Counted::summary`] and [`Counted::rhythm`] in one pass (the "before" column is a
    /// second one, over the earlier days).
    pub fn activity(&self) -> (PeriodSummary, Rhythm) {
        let mut totals = Totals::default();
        let mut series = Series::new(self.axis());
        let mut when = When::default();
        self.reduce(&mut [&mut totals, &mut series, &mut when]);
        let s = self.summary_of(self.scope().period(), totals.finish());
        (s, self.rhythm_of(series, &when))
    }

    fn summary_as(&self, period: Period) -> PeriodSummary {
        self.summary_of(period, self.totals())
    }

    fn summary_of(&self, period: Period, current: PeriodStats) -> PeriodSummary {
        let s = self.scope();
        let (a, b) = (*self.days().start(), *self.days().end());
        let previous = if s.range().is_some() {
            (a > i64::MIN && b < i64::MAX && a <= b).then(|| {
                let n = b - a + 1;
                let before = scope_with(s, period, Some((a - n, a - 1)));
                self.insights().counted(&before, *self.context()).totals()
            })
        } else {
            self.before()
                .map(|p| p.totals())
                .or_else(|| period.days().map(|_| PeriodStats::default()))
        };
        let (from, to) = bounds(self);
        PeriodSummary {
            period,
            from,
            to,
            current,
            previous,
        }
    }

    fn rhythm_of(&self, series: Series, when: &When) -> Rhythm {
        let axis = self.axis();
        let When {
            punch_card,
            by_weekday,
            by_hour,
        } = *when;
        Rhythm {
            series: series.finish(axis),
            unit: axis.unit(),
            punch_max: punch_card.iter().flatten().copied().max().unwrap_or(0),
            punch_card,
            by_weekday,
            by_hour,
            ends: (0..axis.len()).map(|i| axis.end_key(i)).collect(),
        }
    }
}

#[cfg(test)]
mod tests;
