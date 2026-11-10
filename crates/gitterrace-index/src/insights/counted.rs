//! [`Counted`]: the commits one request counts, as a lazy stream over [`Scope::admits`].
//!
//! It holds O(1) state (the scope, the context, the axis, the days and the time span that
//! prunes the scan). Commits stream through [`InsightsSource::commits_in`], their file rows
//! through [`InsightsSource::rows_for`] in chunks of [`CHUNK`] ranks, so neither the commit
//! list nor the row list is ever held whole. [`Counted::ranks`] materializes the rank list only
//! for consumers that need two passes. A `Counted` lives for one request: never cache it, never
//! key a memo by it.
//!
//! Reducers ([`Reducer`]) fold the stream; [`Counted::reduce`] runs several in one pass.

use std::collections::VecDeque;
use std::ops::{Range, RangeInclusive};

use crate::feed::{CommitRange, CommitRow, FileRow, Rank};
use crate::insights::{
    Accel, CountContext, Insights, Label, LabelBucket, LabelCount, Lines, PeriodStats, ReducerKind,
    ReducerOut, Scope, SeriesPoint, SizeBucket, TimeAxis,
};
use crate::seg::SegError;

/// Ranks per `rows_for` call: bounds the rows in flight, not the history.
pub const CHUNK: usize = 4096;

/// Upper bounds (inclusive) of the commit size buckets, in changed lines, as `commit_sizes`.
const SIZE_EDGES: [u32; 6] = [1, 5, 20, 100, 500, 2000];
const SIZE_LABELS: [&str; 7] = ["1", "2-5", "6-20", "21-100", "101-500", "501-2000", "2000+"];

/// The counted commits of one request, see the module docs.
pub struct Counted<'a> {
    ins: &'a Insights<'a>,
    scope: &'a Scope,
    cx: CountContext,
    axis: TimeAxis,
    days: RangeInclusive<i64>,
    span: Range<i64>,
    /// True for [`Counted::before`]: the days are not the scope's own, so no accelerator
    /// (which sees the scope only) may answer.
    shifted: bool,
}

/// One counted commit as a reducer sees it.
#[derive(Debug, Clone)]
pub struct Item {
    pub commit: CommitRow,
    /// Person index after mailmap and identity merging.
    pub person: usize,
    /// Local day, in the commit's own offset.
    pub day: i64,
    /// Local hour 0..24.
    pub hour: usize,
    /// Lines over the counted files (and, with a path selection, the rows under the path).
    pub lines: Lines,
    /// The rows the path selection keeps, path filter not applied. Empty unless a reducer
    /// asked for rows or a path is selected.
    pub rows: Vec<FileRow>,
}

/// A fold over the counted stream.
pub trait Reducer {
    /// True when [`Reducer::add`] reads [`Item::rows`].
    fn needs_rows(&self) -> bool {
        false
    }
    fn add(&mut self, c: &Counted<'_>, item: &Item);
}

impl Insights<'_> {
    /// The commits `scope` counts. `self` is the unscoped history under `scope.options()`,
    /// `cx` built from that same history.
    pub fn counted<'a>(&'a self, scope: &'a Scope, cx: CountContext) -> Counted<'a> {
        Counted::new(self, scope, cx, cx.days(scope), false)
    }
}

impl<'a> Counted<'a> {
    fn new(
        ins: &'a Insights<'a>,
        scope: &'a Scope,
        cx: CountContext,
        days: RangeInclusive<i64>,
        shifted: bool,
    ) -> Self {
        let axis = cx.axis(scope);
        let span = super::scope::span_of(&days);
        Self {
            ins,
            scope,
            cx,
            axis,
            days,
            span,
            shifted,
        }
    }

    /// The same scope one period earlier, for the "before" column. `None` for `All` and for a
    /// `from`/`to` range.
    pub fn before(&self) -> Option<Self> {
        let d = self.scope.period().days()?;
        if self.scope.range().is_some() || self.shifted {
            return None;
        }
        let days = self.days.start().saturating_sub(d)..=self.days.end().saturating_sub(d);
        Some(Self::new(self.ins, self.scope, self.cx, days, true))
    }

    pub const fn scope(&self) -> &Scope {
        self.scope
    }

    pub const fn context(&self) -> &CountContext {
        &self.cx
    }

    pub const fn insights(&self) -> &Insights<'a> {
        self.ins
    }

    pub const fn axis(&self) -> &TimeAxis {
        &self.axis
    }

    /// Local days counted, both inclusive.
    pub const fn days(&self) -> &RangeInclusive<i64> {
        &self.days
    }

    /// Author times scanned: a superset of the days, see [`CountContext::span`].
    pub const fn span(&self) -> &Range<i64> {
        &self.span
    }

    /// [`Scope::admits`] over these days.
    pub fn contains(&self, c: &CommitRow) -> bool {
        self.scope.admits_days(self.ins, &self.days, c)
    }

    /// [`Counted::contains`] without the path selection, for callers that pick the files
    /// themselves: the per-file reports keep only the rows of the head files they show.
    pub(crate) fn contains_columns(&self, c: &CommitRow) -> bool {
        self.scope.admits_in(self.ins, &self.days, c)
    }

    /// The counted commits, oldest rank first.
    pub fn stream(&self) -> impl Iterator<Item = CommitRow> + '_ {
        self.items(false).map(|i| i.commit)
    }

    /// The counted commits with the rows the selection keeps, oldest rank first. The commit
    /// row comes along, so a reader never looks a commit up again by rank.
    pub fn rows(&self) -> impl Iterator<Item = (CommitRow, Vec<FileRow>)> + '_ {
        self.items(true).map(|i| (i.commit, i.rows))
    }

    /// Ranks of the counted commits, ascending: for two-pass consumers only.
    pub fn ranks(&self) -> Vec<Rank> {
        self.stream().map(|c| c.rank).collect()
    }

    /// Feeds every counted commit to every reducer, in one pass.
    pub fn reduce(&self, reducers: &mut [&mut dyn Reducer]) {
        let rows = reducers.iter().any(|r| r.needs_rows());
        for item in self.items(rows) {
            for r in reducers.iter_mut() {
                r.add(self, &item);
            }
        }
    }

    /// `kind` from `accel` when it can prove the answer, else from the stream. A read error of
    /// the accelerator is returned, never replaced by the stream.
    pub fn reduce_kind(
        &self,
        accel: &dyn Accel,
        kind: ReducerKind,
    ) -> Result<ReducerOut, SegError> {
        if !self.shifted
            && let Some(out) = accel.try_reduce(self.scope, &self.cx, kind)
        {
            return out;
        }
        Ok(self.stream_kind(kind))
    }

    /// `kind` from the stream, whatever an accelerator could say.
    pub fn stream_kind(&self, kind: ReducerKind) -> ReducerOut {
        match kind {
            ReducerKind::Totals => ReducerOut::Totals(self.totals()),
            ReducerKind::Series => ReducerOut::Series(self.series()),
            ReducerKind::When => ReducerOut::When(Box::new(self.when())),
            ReducerKind::PerPerson => ReducerOut::PerPerson(self.per_person()),
            ReducerKind::PerLabel(l) => {
                let (a, b) = self.per_label(l);
                ReducerOut::PerLabel(a, b)
            }
            ReducerKind::Sizes => ReducerOut::Sizes(self.sizes()),
        }
    }

    pub fn totals(&self) -> PeriodStats {
        self.one(Totals::of(self)).finish()
    }

    /// True for every day of the history with no selection: the set the shared tables
    /// already summarize.
    pub(crate) fn is_whole(&self) -> bool {
        !self.shifted
            && self.scope.selection().is_empty()
            && *self.days.start() == i64::MIN
            && *self.days.end() == i64::MAX
    }

    pub fn series(&self) -> Vec<SeriesPoint> {
        self.one(Series::new(&self.axis)).finish(&self.axis)
    }

    pub fn when(&self) -> When {
        self.one(When::default())
    }

    pub fn per_person(&self) -> Vec<PersonTally> {
        self.one(PersonTallies::new(&self.axis)).finish(self)
    }

    pub fn per_label(&self, label: Label) -> (Vec<LabelCount>, Vec<LabelBucket>) {
        self.one(LabelTally::new(label, &self.axis)).finish(self)
    }

    pub fn sizes(&self) -> Vec<SizeBucket> {
        self.one(Sizes::default()).finish()
    }

    fn one<R: Reducer>(&self, mut r: R) -> R {
        self.reduce(&mut [&mut r]);
        r
    }

    /// The stream behind every public reader. Rows are read in chunks of [`CHUNK`] ranks, only
    /// when asked for or when a path selection must see them.
    fn items(&self, with_rows: bool) -> impl Iterator<Item = Item> + '_ {
        let src = self.ins.source();
        let path = self.scope.selection().path().is_some();
        let need = with_rows || path;
        self.ins.count_walk(need);
        let mut base = src
            .commits_in(CommitRange::Time(self.span.clone()))
            .filter(move |c| self.scope.admits_in(self.ins, &self.days, c));
        let mut buf: VecDeque<Item> = VecDeque::new();
        std::iter::from_fn(move || {
            loop {
                if let Some(x) = buf.pop_front() {
                    return Some(x);
                }
                let chunk: Vec<CommitRow> = base.by_ref().take(CHUNK).collect();
                if chunk.is_empty() {
                    return None;
                }
                if !need {
                    buf.extend(chunk.into_iter().map(|c| self.item(c, Vec::new(), None)));
                    continue;
                }
                let ranks: Vec<Rank> = chunk.iter().map(|c| c.rank).collect();
                let mut by = chunk.into_iter();
                for (r, rows) in src.rows_for(ranks) {
                    let Some(c) = by.find(|c| c.rank == r) else {
                        continue;
                    };
                    // A merge counts as a commit only: no files, no lines.
                    let kept: Vec<FileRow> = if c.merge {
                        Vec::new()
                    } else {
                        self.scope.keeps_rows(src, &rows).copied().collect()
                    };
                    if path && kept.is_empty() {
                        continue;
                    }
                    let lines = path.then(|| self.lines_of(&kept));
                    buf.push_back(self.item(c, kept, lines));
                }
            }
        })
    }

    fn item(&self, c: CommitRow, rows: Vec<FileRow>, lines: Option<Lines>) -> Item {
        Item {
            person: self.ins.person_of(c.ident),
            day: crate::feed::local_day_of(c.time, c.offset),
            hour: self.cx.local_hour(c.time, c.offset),
            lines: lines.unwrap_or_else(|| self.ins.commit_sums(&c)),
            rows,
            commit: c,
        }
    }

    fn lines_of(&self, rows: &[FileRow]) -> Lines {
        rows.iter()
            .filter_map(|r| self.ins.row_lines(r))
            .fold(Lines::default(), |mut o, l| {
                o.add += l.add;
                o.del += l.del;
                o.rewritten += l.rewritten;
                o
            })
    }
}

mod reducers;

pub use reducers::{LabelTally, PersonTallies, PersonTally, Series, Sizes, Totals, When};

#[cfg(test)]
mod tests {
    use crate::feed::{FixedClock, MemorySource};
    use crate::insights::fixtures::{MON, chain};
    use crate::insights::{
        CountContext, Insights, Label, NoAccel, Options, Period, ReducerKind, ReducerOut, Scope,
        ScopeParams,
    };

    fn scope(period: &str, path: Option<&str>) -> Scope {
        let mut p = ScopeParams {
            period: Some(period.to_owned()),
            ..ScopeParams::default()
        };
        p.select.path = path.map(str::to_owned);
        Scope::from_params(&p, Options::default(), Period::All).expect("scope")
    }

    #[test]
    fn reducers_agree_on_the_counted_set() {
        let recs = chain();
        let ins = Insights::new(&recs, Options::default(), None);
        let cx = CountContext::new(&MemorySource::new(&recs), &FixedClock(MON + 100 * 86_400));
        for s in [
            scope("all", None),
            scope("1m", None),
            scope("all", Some("x.rs")),
        ] {
            let c = ins.counted(&s, cx);
            let t = c.totals();
            assert_eq!(t.commits, c.ranks().len());
            assert_eq!(
                t.commits,
                c.series().iter().map(|p| p.commits as usize).sum::<usize>()
            );
            assert_eq!(
                t.commits,
                c.per_person().iter().map(|p| p.commits).sum::<usize>()
            );
            assert_eq!(
                t.commits,
                c.per_label(Label::Kind)
                    .0
                    .iter()
                    .map(|l| l.commits)
                    .sum::<usize>()
            );
            assert_eq!(
                t.commits,
                c.sizes().iter().map(|b| b.commits).sum::<usize>()
            );
            assert_eq!(t.commits as u32, c.when().by_hour.iter().sum::<u32>());
            for k in ReducerKind::ALL {
                assert_eq!(c.reduce_kind(&NoAccel, k).ok(), Some(c.stream_kind(k)));
            }
            assert!(matches!(
                c.stream_kind(ReducerKind::Totals),
                ReducerOut::Totals(_)
            ));
        }
        // All four commits, a path selection keeps the three touching x.rs with their lines.
        let all = ins.counted(&scope("all", None), cx).totals();
        assert_eq!((all.commits, all.authors, all.files), (4, 2, 2));
        let x = ins.counted(&scope("all", Some("x.rs")), cx).totals();
        assert_eq!(
            (x.commits, x.additions, x.deletions, x.files),
            (3, 31, 3, 1)
        );
    }

    #[test]
    fn before_shifts_by_one_period() {
        let recs = chain();
        let ins = Insights::new(&recs, Options::default(), None);
        let cx = CountContext::new(&MemorySource::new(&recs), &FixedClock(MON + 100 * 86_400));
        let s = scope("1w", None);
        let c = ins.counted(&s, cx);
        let b = c.before().expect("fixed period");
        assert_eq!(b.days().end() + 7, *c.days().end());
        assert!(b.before().is_none());
        assert!(ins.counted(&scope("all", None), cx).before().is_none());
    }
}
