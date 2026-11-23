//! What one statistics request counts: [`Scope`] (period, day range, selection, options) and
//! [`CountContext`] (the request's "now" and the local day every period ends on). Together
//! they decide membership through [`Scope::admits`], the one rule every count is meant to
//! follow: a commit counts when [`Insights::counted_row`] keeps it, the selection keeps it and
//! its local day (the author's own offset) lies in the scope's days.
//!
//! Day math lives here and in [`crate::feed::local_day_of`], nowhere else.
//!
//! # Counting rules
//!
//! Each page reads one `Counted` per request, so two numbers on one page never disagree about
//! which commits they cover. The rules every count follows:
//!
//! - Merges are left out by default: a merge's diff against its first parent repeats the lines
//!   its branch already brought. `merges=1` counts them as commits, through [`Scope::admits`],
//!   so every section follows it together. A merge's lines and files are always zero, and a
//!   path selection never keeps a merge. The overview and the sidebar show every commit,
//!   merges and bots included. The overview says how many are merges, how many bots wrote
//!   and how many count, and the API's `/repo` gives both: `every_commits` for every commit,
//!   `commits` for the counted ones.
//! - A bot's commit (name or e-mail marks a bot, such as `dependabot[bot]`) counts only with
//!   `insights.include_bots` or `bots=1`, on every page and list. A file's history and the
//!   sidebar's hot files follow it too, so a file's history lists the commits its hotspots
//!   row counts.
//! - Authors are people after `.mailmap`. Ranked people with equal values order by display
//!   name, then e-mail, never by the order the index first met them.
//! - A contributor is new in a period when their first counted commit over the whole history
//!   lies in that period. The options (merges, bots) apply and the selection does not: a
//!   filter chip never turns a long-time contributor into a newcomer, and a merge or an
//!   uncounted bot commit never makes anyone a returning contributor.
//! - A commit's day is its author's local day, in the author's own offset, for bars, punch
//!   card, weekday split and totals. The commit log alone groups by the committer's local day,
//!   so its order and headings agree, while its counts still use the author's day.
//! - A period is the last 7, 30, 90 or 365 local days ending on the newest commit's local day.
//!   A selection never moves that end. `from` and `to` are inclusive local days.
//!   The Languages card is the one exception: it compares lines before and after a
//!   contiguous stretch of the first-parent history, so its period runs from the first chain
//!   position whose local day is in the period to the newest commit. A later commit with an
//!   older author date is included, and a period with no commit shows an empty card.
//! - `all` is every commit: one dated after the server's clock still counts, at most a day
//!   ahead of the clock that indexed it, so the same history gives the same totals.
//! - Code age and release cadence cover the whole history whatever the period. A file's age is
//!   its last counted change, and the commits between releases are the counted ones.
//! - A binary file changes with 0 lines but still counts as a changed file and a hotspot change.
//! - Ownership and the per-file pages follow a renamed file across its earlier names, as
//!   `git log --follow` does. Hotspots, the all-time ownership totals and "Files changed" count
//!   each name on its own.

use std::ops::{Range, RangeInclusive};

use crate::feed::{self, Clock, CommitRange, CommitRow, FileRow, InsightsSource};
use crate::insights::{DAY, Insights, OptionParams, Options, Period, Selection, TimeAxis};
use crate::query::{Leaf, Matcher, When};

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
    #[error("{0}")]
    Query(#[from] crate::query::QueryError),
    #[error("statistics cannot select by message text: use the log for {0:?}")]
    Words(String),
}

/// One request's counting question. Built only by [`Scope::from_params`], so every surface
/// that reads the same query string asks the same question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    period: Period,
    range: Option<(i64, i64)>,
    /// The selection as canonical filter text, for the scopes derived from this one.
    sel: Selection,
    m: Matcher,
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
        let q = p.select.query()?;
        // A period in the text (`after:1m`) is the scope's period, which the
        // pages and the buckets follow: the same as the `period` parameter, which says it only
        // when the parameter does not. Date bounds stay terms of the matcher.
        let lifted = q.terms().iter().find_map(|t| match t.leaf {
            Leaf::After(When::Period(w)) if !t.neg => Some(w),
            _ => None,
        });
        let period = match (set(p.period.as_deref()), lifted) {
            (None, Some(w)) => w,
            _ => period,
        };
        let q = q.replace(|l| matches!(l, Leaf::After(When::Period(_))), None);
        let m = Matcher::new(&q);
        if let Some((_, w)) = m.words().first() {
            return Err(ScopeError::Words(w.clone()));
        }
        Ok(Self { period, range, sel: Selection::of(&q), m, opts: p.options.apply(defaults) })
    }

    pub const fn period(&self) -> Period {
        self.period
    }

    /// The `from`/`to` local days, both inclusive, when the URL narrows the period.
    pub const fn range(&self) -> Option<(i64, i64)> {
        self.range
    }

    /// The commit filter this scope counts, as canonical text: its period (as `after:`, none
    /// for `all`) and then its selection, whichever way the request said them.
    pub fn query_text(&self) -> String {
        let period = (self.period != Period::All).then(|| format!("after:{}", self.period));
        let sel = self.sel.q.clone();
        [period, sel].into_iter().flatten().collect::<Vec<_>>().join(" ")
    }

    pub const fn selection(&self) -> &Selection {
        &self.sel
    }

    /// The selection compiled: what every count tests commits and rows with.
    pub const fn matcher(&self) -> &Matcher {
        &self.m
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
            && (!self.m.has_path() || {
                // A merge has no files of its own, so a path selection never keeps one.
                !c.merge && {
                    let src = ins.source();
                    src.rows_for(vec![c.rank])
                        .next()
                        .is_some_and(|(_, rows)| self.keeps_rows(src, &rows).next().is_some())
                }
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
            && self.selects(ins.source(), c, ins.now)
    }

    /// The selection's column terms over a commit, its bounds counted back from `now`.
    pub(crate) fn selects(&self, src: &dyn InsightsSource, c: &CommitRow, now: i64) -> bool {
        self.m.commit_row(src, c, now)
    }

    /// The rows of one commit a path selection keeps: all of them without one.
    pub(crate) fn keeps_rows<'r>(
        &self,
        src: &'r dyn InsightsSource,
        rows: &'r [FileRow],
    ) -> impl Iterator<Item = &'r FileRow> + 'r {
        let m = self.m.clone();
        let all = !m.has_path();
        rows.iter().filter(move |f| all || src.path(f.path).is_some_and(|n| m.row(&n)))
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

/// The period anchors of a shared set, memoized there: the one place besides a request's
/// entry that builds a [`CountContext`], since every page of the set reads the same `now`.
impl Insights<'_> {
    /// [`CountContext::new`] over this history at this instance's clock, built once per set.
    /// Every `Insights` over one shared set reads the same `now` (`with` derives it from the
    /// same source), so the context is the same for all of them.
    pub fn context(&self) -> CountContext {
        if let Ok(g) = self.d.context.lock()
            && let Some((now, cx)) = *g
            && now == self.now
        {
            return cx;
        }
        let cx = CountContext::new(self.source(), &feed::FixedClock(self.now));
        if let Ok(mut g) = self.d.context.lock() {
            *g = Some((self.now, cx));
        }
        cx
    }

    /// The context `CountContext::new(src, clock)` gives, from the memo when `src` is this
    /// history and `clock` reads the instant this instance was opened at.
    pub fn context_for(&self, src: &dyn InsightsSource, clock: &dyn Clock) -> CountContext {
        let same = std::ptr::addr_eq(std::ptr::from_ref(src), std::ptr::from_ref(self.source()));
        // With the clock past the newest commit, the reference time is that commit's: no scan.
        let newest = self.d.newest.get().copied();
        if same && newest == Some(self.now) && clock.now() >= self.now && self.now >= 0 {
            return self.context();
        }
        CountContext::new(src, clock)
    }
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
            (Some(anchor_day), Some(first_day), Some(last_day)) => {
                Self { now, anchor_day, first_day, last_day }
            }
            _ => Self { now, anchor_day: 0, first_day: 1, last_day: 0 },
        }
    }

    /// The newest author time not after the clock: [`crate::feed::reference_time`].
    pub const fn now(&self) -> i64 {
        self.now
    }

    /// Local day of the commit that sets [`CountContext::now`]: every period ends on it.
    pub const fn anchor_day(&self) -> i64 {
        self.anchor_day
    }

    /// Local day of the oldest commit; after [`CountContext::last_day`] for an empty history.
    pub const fn first_day(&self) -> i64 {
        self.first_day
    }

    /// Newest local day of the history, see the field.
    pub const fn last_day(&self) -> i64 {
        self.last_day
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
            (self.anchor_day.saturating_add(1).saturating_sub(d), self.anchor_day)
        });
        let (lo, hi) = s.range.unwrap_or((i64::MIN, i64::MAX));
        first.max(lo)..=last.min(hi)
    }

    /// The chart axis of `s`: its period's buckets, or with a range the range's own days.
    pub fn axis(&self, s: &Scope) -> TimeAxis {
        if self.last_day < self.first_day {
            return TimeAxis::for_period(Some(s.period), 1, 0);
        }
        let last = if s.period.days().is_some() { self.anchor_day } else { self.last_day };
        if s.range.is_none() {
            return TimeAxis::for_period(Some(s.period), self.first_day, last);
        }
        let d = self.days(s);
        TimeAxis::for_period(None, (*d.start()).max(self.first_day), (*d.end()).min(self.last_day))
    }

    /// Author times that may hold one of `s`'s local days: the days widened by two days on
    /// each side, more than any offset git stores. Only prunes a scan, never decides.
    pub fn span(&self, s: &Scope) -> Range<i64> {
        span_of(&self.days(s))
    }
}

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
        days.end().saturating_add(1).saturating_mul(DAY).saturating_add(margin)
    };
    lo..hi
}

fn set(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

/// `YYYY-MM-DD` as a day number, `None` for anything else (including 2026-02-30).
pub fn parse_day(s: &str) -> Option<i64> {
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
    best.or(oldest).map(|c| feed::local_day_of(c.time, c.offset))
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
                "q" => p.select.q = v,
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
    fn a_period_in_the_text_is_the_period_parameter() {
        for (a, b) in [
            (&[("period", "1m")][..], &[("q", "after:1m")][..]),
            (&[("period", "1w")], &[("q", "after:1w")]),
        ] {
            let (a, b) = (scope(a).expect("a"), scope(b).expect("b"));
            assert_eq!(a, b, "{a:?}");
        }
        // The parameter wins over the text, as the web's filter has it.
        let both = scope(&[("period", "1y"), ("q", "after:1m")]).expect("both");
        assert_eq!(both.period(), Period::Year);
    }

    #[test]
    fn from_params_normalises_and_validates() {
        let s = scope(&[("q", "  path:/src/ ")]).expect("ok");
        assert_eq!(s.period(), Period::Month);
        assert_eq!(s.selection().q.as_deref(), Some("path:src"));
        assert!(matches!(scope(&[("q", "a | b")]), Err(ScopeError::Query(_))));
        assert!(matches!(scope(&[("q", "fix")]), Err(ScopeError::Words(_))));
        assert!(matches!(scope(&[("period", "2w")]), Err(ScopeError::Period(_))));
        assert!(matches!(scope(&[("from", "2026-02-30")]), Err(ScopeError::Date(..))));
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

    mod merge {
        use super::{CountContext, Scope, ScopeParams};
        use crate::feed::{CommitRange, FixedClock, InsightsSource, MemorySource};
        use crate::insights::fixtures::{MON, chain};
        use crate::insights::{Insights, Options, Period};

        #[test]
        fn admits_a_merge_only_with_the_option() {
            let mut recs = chain();
            recs[2].merge = true;
            let src = MemorySource::new(&recs);
            let cx = CountContext::new(&src, &FixedClock(MON + 1000 * 86_400));
            let count = |merges: bool| {
                let o =
                    Options { include_merges: merges, include_bots: true, ..Options::default() };
                let ins = Insights::new(&recs, o.clone(), None);
                let s = Scope::from_params(&ScopeParams::default(), o, Period::All).expect("scope");
                let n = src.commits_in(CommitRange::All).filter(|c| s.admits(&ins, &cx, c)).count();
                let lines = ins.counted(&s, cx).totals();
                (n, lines.commits, lines.additions, lines.files)
            };
            let (off, on) = (count(false), count(true));
            assert_eq!(on.0, off.0 + 1, "the merge is admitted with the option");
            assert_eq!(on.1, off.1 + 1, "and counted as a commit");
            assert_eq!((on.2, on.3), (off.2, off.3), "lines and files are unchanged");
        }
    }
}
