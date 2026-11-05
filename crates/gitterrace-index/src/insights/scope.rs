//! What one statistics request counts: [`Scope`] (period, day range, selection, options) and
//! [`CountContext`] (the request's "now" and the local day every period ends on). Together
//! they decide membership through [`Scope::admits`], the one rule every count is meant to
//! follow: a commit counts when [`Insights::counted_row`] keeps it, the selection keeps it and
//! its local day (the author's own offset) lies in the scope's days.
//!
//! Day math lives here and in [`crate::feed::local_day_of`], nowhere else.

use std::ops::{Range, RangeInclusive};

use crate::feed::{self, Clock, CommitRange, CommitRow, FileRow, InsightsSource};
use crate::insights::{DAY, Insights, OptionParams, Options, Period, Selection, TimeAxis, under};

/// A scope as a query string carries it: every value a plain, possibly empty or junk, string.
/// The web pages and the API both fill this and call [`Scope::from_params`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct ScopeParams {
    /// `1w`, `1m`, `3m`, `1y` or `all`.
    pub period: Option<String>,
    /// First local day, `YYYY-MM-DD`.
    pub from: Option<String>,
    /// Last local day, `YYYY-MM-DD`, inclusive.
    pub to: Option<String>,
    #[serde(flatten)]
    pub select: Selection,
    #[serde(flatten)]
    pub options: OptionParams,
}

/// Why a [`ScopeParams`] was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    #[error("period must be 1w, 1m, 3m, 1y or all, got {0}")]
    Period(String),
    #[error("{0} must be a date as YYYY-MM-DD, got {1}")]
    Date(&'static str, String),
    #[error("from {0} is after to {1}")]
    Order(String, String),
}

/// One request's counting question. Built only by [`Scope::from_params`], so every surface
/// that reads the same query string asks the same question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    period: Period,
    /// Local days, both inclusive.
    range: Option<(i64, i64)>,
    sel: Selection,
    opts: Options,
}

impl Scope {
    /// Validates and normalises `p`. `defaults` are the configured options the URL overrides,
    /// `period` the page's own default period when the URL names none.
    pub fn from_params(
        p: &ScopeParams,
        defaults: Options,
        period: Period,
    ) -> Result<Self, ScopeError> {
        let period = match set(p.period.as_deref()) {
            None => period,
            Some(s) => s.parse().map_err(|_| ScopeError::Period(s.to_owned()))?,
        };
        let day = |what: &'static str, v: Option<&str>| {
            set(v)
                .map(|s| parse_day(s).ok_or_else(|| ScopeError::Date(what, s.to_owned())))
                .transpose()
        };
        let (from, to) = (day("from", p.from.as_deref())?, day("to", p.to.as_deref())?);
        let range = match (from, to) {
            (None, None) => None,
            (Some(f), Some(t)) if f > t => {
                return Err(ScopeError::Order(
                    crate::insights::format_date(f),
                    crate::insights::format_date(t),
                ));
            }
            (f, t) => Some((f.unwrap_or(i64::MIN), t.unwrap_or(i64::MAX))),
        };
        let norm = |v: Option<&String>| set(v.map(String::as_str)).map(str::to_owned);
        let sel = Selection {
            kind: norm(p.select.kind.as_ref()),
            scope: norm(p.select.scope.as_ref()),
            author: norm(p.select.author.as_ref()),
            path: p.select.path().map(str::to_owned),
        };
        Ok(Self {
            period,
            range,
            sel,
            opts: p.options.apply(defaults),
        })
    }

    pub const fn period(&self) -> Period {
        self.period
    }

    /// The `from`/`to` local days, both inclusive, when the URL narrows the period.
    pub const fn range(&self) -> Option<(i64, i64)> {
        self.range
    }

    pub const fn selection(&self) -> &Selection {
        &self.sel
    }

    pub const fn options(&self) -> &Options {
        &self.opts
    }

    /// The membership rule, see the module docs. `ins` must be over the unscoped history and
    /// built with [`Scope::options`]; `cx` from that same history. [`Insights::counted_row`]
    /// is its first half, the local day and the selection the rest.
    pub fn admits(&self, ins: &Insights<'_>, cx: &CountContext, c: &CommitRow) -> bool {
        self.admits_days(ins, &cx.days(self), c)
    }

    /// [`Scope::admits`] over explicit local `days` (the "before" column shifts them).
    pub(crate) fn admits_days(
        &self,
        ins: &Insights<'_>,
        days: &RangeInclusive<i64>,
        c: &CommitRow,
    ) -> bool {
        self.admits_in(ins, days, c)
            && self.sel.path().is_none_or(|_| {
                let src = ins.source();
                src.rows_for(vec![c.rank])
                    .next()
                    .is_some_and(|(_, rows)| self.keeps_rows(src, &rows).next().is_some())
            })
    }

    /// [`Scope::admits_days`] without the path selection, which needs the file rows: the
    /// stream checks the path on rows it reads anyway.
    pub(crate) fn admits_in(
        &self,
        ins: &Insights<'_>,
        days: &RangeInclusive<i64>,
        c: &CommitRow,
    ) -> bool {
        ins.counted_row(c)
            && days.contains(&feed::local_day_of(c.time, c.offset))
            && self.selects(ins.source(), c)
    }

    /// The type, scope and author selection over a commit's columns.
    pub(crate) fn selects(&self, src: &dyn InsightsSource, c: &CommitRow) -> bool {
        let label_is = |id: Option<crate::feed::LabelId>, want: &str, none: &str| {
            id.and_then(|l| src.label(l)).as_deref().unwrap_or(none) == want
        };
        self.sel
            .kind()
            .is_none_or(|k| label_is(c.kind, k, Selection::OTHER))
            && self
                .sel
                .scope()
                .is_none_or(|s| label_is(c.scope, s, Selection::NO_SCOPE))
            && self.sel.author().is_none_or(|a| {
                src.ident(c.ident)
                    .is_some_and(|(name, email)| email.eq_ignore_ascii_case(a) || &*name == a)
            })
    }

    /// The rows of one commit a path selection keeps: all of them without one.
    pub(crate) fn keeps_rows<'r>(
        &self,
        src: &'r dyn InsightsSource,
        rows: &'r [FileRow],
    ) -> impl Iterator<Item = &'r FileRow> + 'r {
        let path = self.sel.path().map(str::to_owned);
        rows.iter().filter(move |f| {
            path.as_deref()
                .is_none_or(|p| src.path(f.path).is_some_and(|n| under(&n, p)))
        })
    }
}

/// The request's clock reading and anchors, taken from the UNSCOPED history: a selection
/// never moves the week "last week" means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountContext {
    now: i64,
    anchor_day: i64,
    first_day: i64,
    /// Newest local day of the history: an earlier commit in a later offset can sit a day
    /// after the anchor, and `All` must still count it.
    last_day: i64,
}

impl CountContext {
    /// Reads `clock` once. An empty history anchors on day 0 and has no days.
    pub fn new(unscoped: &dyn InsightsSource, clock: &dyn Clock) -> Self {
        let now = feed::reference_time_at(unscoped, clock);
        let anchor_day = reference_day_at(unscoped, now);
        let (mut lo, mut hi) = (None::<i64>, None::<i64>);
        for c in unscoped.commits_in(CommitRange::All) {
            let d = feed::local_day_of(c.time, c.offset);
            lo = Some(lo.map_or(d, |x| x.min(d)));
            hi = Some(hi.map_or(d, |x| x.max(d)));
        }
        match (anchor_day, lo, hi) {
            (Some(anchor_day), Some(first_day), Some(last_day)) => Self {
                now,
                anchor_day,
                first_day,
                last_day,
            },
            _ => Self {
                now,
                anchor_day: 0,
                first_day: 1,
                last_day: 0,
            },
        }
    }

    /// The newest author time not after the clock: [`crate::reference_time`].
    pub const fn now(&self) -> i64 {
        self.now
    }

    /// Local day of the commit that sets [`CountContext::now`]: every period ends on it.
    pub const fn anchor_day(&self) -> i64 {
        self.anchor_day
    }

    /// Days since 1970-01-01 in the commit's own offset.
    pub const fn local_day(&self, time: i64, offset: i32) -> i64 {
        feed::local_day_of(time, offset)
    }

    /// Hour 0..24 in the commit's own offset.
    pub fn local_hour(&self, time: i64, offset: i32) -> usize {
        usize::try_from(feed::local_time_of(time, offset).rem_euclid(DAY) / 3600).unwrap_or(0)
    }

    /// Whole days from instant `a` to instant `b`, rounded down: release gaps and ages.
    pub const fn days_between(&self, a: i64, b: i64) -> i64 {
        b.saturating_sub(a).div_euclid(DAY)
    }

    /// The local days `s` counts. A fixed period is the last `days` local days ending on the
    /// anchor day, `All` every day. A `from`/`to` range narrows either.
    pub fn days(&self, s: &Scope) -> RangeInclusive<i64> {
        let (first, last) = s.period.days().map_or((i64::MIN, i64::MAX), |d| {
            (
                self.anchor_day.saturating_add(1).saturating_sub(d),
                self.anchor_day,
            )
        });
        let (lo, hi) = s.range.unwrap_or((i64::MIN, i64::MAX));
        first.max(lo)..=last.min(hi)
    }

    /// The chart axis of `s`: its period's buckets, or with a range the range's own days.
    pub fn axis(&self, s: &Scope) -> TimeAxis {
        if self.last_day < self.first_day {
            return TimeAxis::for_period(Some(s.period), 1, 0);
        }
        let last = if s.period.days().is_some() {
            self.anchor_day
        } else {
            self.last_day
        };
        if s.range.is_none() {
            return TimeAxis::for_period(Some(s.period), self.first_day, last);
        }
        let d = self.days(s);
        TimeAxis::for_period(
            None,
            (*d.start()).max(self.first_day),
            (*d.end()).min(self.last_day),
        )
    }

    /// Author times that may hold one of `s`'s local days: the days widened by two days on
    /// each side, more than any offset git stores. Only prunes a scan, never decides.
    pub fn span(&self, s: &Scope) -> Range<i64> {
        span_of(&self.days(s))
    }
}

/// [`CountContext::span`] of explicit days.
pub(crate) const fn span_of(days: &RangeInclusive<i64>) -> Range<i64> {
    let margin = 2 * DAY;
    let lo = if *days.start() == i64::MIN {
        i64::MIN
    } else {
        days.start().saturating_mul(DAY).saturating_sub(margin)
    };
    let hi = if *days.end() == i64::MAX {
        i64::MAX
    } else {
        days.end()
            .saturating_add(1)
            .saturating_mul(DAY)
            .saturating_add(margin)
    };
    lo..hi
}

fn set(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

/// `YYYY-MM-DD` as a day number, `None` for anything else (including 2026-02-30).
fn parse_day(s: &str) -> Option<i64> {
    let mut it = s.splitn(3, '-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    let day = crate::insights::days_from_civil(y, m, d);
    (gitterrace_git::civil_from_days(day) == (y, m, d)).then_some(day)
}

/// [`reference_day`] for a "now" already read.
pub(super) fn reference_day_at(src: &dyn InsightsSource, now: i64) -> Option<i64> {
    let (mut best, mut oldest): (Option<CommitRow>, Option<CommitRow>) = (None, None);
    for c in src.commits_in(CommitRange::All) {
        if c.time <= now && best.is_none_or(|b| c.time >= b.time) {
            best = Some(c);
        }
        if oldest.is_none_or(|o| c.time < o.time) {
            oldest = Some(c);
        }
    }
    best.or(oldest)
        .map(|c| feed::local_day_of(c.time, c.offset))
}

#[cfg(test)]
mod tests {
    use super::{CountContext, Scope, ScopeError, ScopeParams};
    use crate::feed::{FixedClock, MemorySource};
    use crate::insights::fixtures::{MON, chain};
    use crate::insights::{Options, Period, days_from_civil};

    fn params(q: &[(&str, &str)]) -> ScopeParams {
        let mut p = ScopeParams::default();
        for (k, v) in q {
            let v = Some((*v).to_owned());
            match *k {
                "period" => p.period = v,
                "from" => p.from = v,
                "to" => p.to = v,
                "author" => p.select.author = v,
                "path" => p.select.path = v,
                "bots" => p.options.bots = v,
                _ => {}
            }
        }
        p
    }

    fn scope(q: &[(&str, &str)]) -> Result<Scope, ScopeError> {
        Scope::from_params(&params(q), Options::default(), Period::Month)
    }

    #[test]
    fn from_params_normalises_and_validates() {
        let s = scope(&[("author", "  "), ("path", "/src/")]).expect("ok");
        assert_eq!(s.period(), Period::Month);
        assert_eq!(s.selection().author, None);
        assert_eq!(s.selection().path.as_deref(), Some("src"));
        assert!(matches!(
            scope(&[("period", "2w")]),
            Err(ScopeError::Period(_))
        ));
        assert!(matches!(
            scope(&[("from", "2026-02-30")]),
            Err(ScopeError::Date(..))
        ));
        assert!(matches!(
            scope(&[("from", "2026-03-02"), ("to", "2026-03-01")]),
            Err(ScopeError::Order(..))
        ));
        let r = scope(&[("from", "2026-03-01")]).expect("ok").range();
        assert_eq!(r, Some((days_from_civil(2026, 3, 1), i64::MAX)));
        assert!(scope(&[("bots", "1")]).expect("ok").options().include_bots);
    }

    #[test]
    fn days_end_on_the_anchor_of_the_unscoped_history() {
        let recs = chain();
        let src = MemorySource::new(&recs);
        let cx = CountContext::new(&src, &FixedClock(MON + 1000 * 86_400));
        let last = (MON + 40 * 86_400 + 12 * 3600).div_euclid(86_400);
        assert_eq!(cx.anchor_day(), last);
        let w = scope(&[("period", "1w")]).expect("ok");
        assert_eq!(cx.days(&w), last - 6..=last);
        assert_eq!(cx.axis(&w).len(), 7);
        // A clock before the newest commit moves the anchor back.
        let early = CountContext::new(&src, &FixedClock(MON + 35 * 86_400));
        assert_eq!(early.anchor_day(), last - 10);
    }
}
