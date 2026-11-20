use crate::insights::{BucketUnit, Counted, Insights, Item, Lines, Period, Reducer, TimeAxis};

/// Additions and deletions of one bucket, moves and (by default) whitespace excluded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Frequency {
    pub key: String,
    pub label: String,
    pub additions: u64,
    pub deletions: u64,
    pub rewritten: u64,
}

/// Lines of every counted commit, whatever bucket it falls in: the KPIs next to the bars.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct LineTotals {
    pub additions: u64,
    pub deletions: u64,
    pub rewritten: u64,
}

/// Rework over a rolling window of commits, read at the end of each bucket.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ReworkPoint {
    pub key: String,
    pub label: String,
    pub added: u64,
    pub removed: u64,
    pub rewritten: u64,
    /// Share of changed lines that replaced existing code. `None` when nothing changed.
    pub share: Option<f64>,
}

/// The rework series with the window it was computed over.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Rework {
    pub window: usize,
    pub unit: BucketUnit,
    pub points: Vec<ReworkPoint>,
}

/// `2 * rewritten / (added + removed)`: 0 for purely new code, 1 when every change replaced
/// existing lines.
pub fn rework_share(added: u64, removed: u64, rewritten: u64) -> Option<f64> {
    (added + removed > 0).then(|| 2.0 * rewritten as f64 / (added + removed) as f64)
}

pub fn rework_verdict(share: f64) -> &'static str {
    if share < 0.25 {
        "mostly new code"
    } else if share < 0.6 {
        "growing while reworking"
    } else {
        "mostly reworking existing code"
    }
}

#[expect(
    clippy::redundant_closure_for_method_calls,
    reason = "a method path binds one Counted lifetime, in_period needs a closure over any"
)]
impl Insights<'_> {
    pub fn frequency(&self, period: Period) -> (BucketUnit, Vec<Frequency>) {
        self.in_period(period, |c| c.frequency())
    }

    /// Rolling rework read at the end of every bucket of `period`, see [`Counted::rework`].
    pub fn rework(&self, period: Period) -> Rework {
        self.in_period(period, |c| c.rework())
    }
}

impl Counted<'_> {
    /// Lines per axis bucket over the counted commits.
    pub fn frequency(&self) -> (BucketUnit, Vec<Frequency>) {
        let (unit, out, _) = self.frequency_totals();
        (unit, out)
    }

    /// [`Counted::frequency`] and the lines of every counted commit, in one pass. The totals
    /// never come from the buckets: a commit outside the axis still counts in them.
    pub fn frequency_totals(&self) -> (BucketUnit, Vec<Frequency>, LineTotals) {
        let mut r = FrequencyTally::new(self.axis());
        self.reduce(&mut [&mut r]);
        let (unit, out, t, _) = r.finish(self);
        (unit, out, t)
    }

    /// Rolling rework read at the end of every bucket. The window is an eighth of the counted
    /// commits, between 10 and 100: a week of a dozen commits must not average over a hundred
    /// older ones, and a big history needs the 100 to smooth out.
    pub fn rework(&self) -> Rework {
        let mut r = FrequencyTally::new(self.axis());
        self.reduce(&mut [&mut r]);
        r.finish(self).3
    }
}

/// Code frequency, its line totals and the rolling rework, from one pass: see
/// [`Counted::frequency_totals`] and [`Counted::rework`].
pub struct FrequencyTally {
    out: Vec<Frequency>,
    sum: LineTotals,
    /// Bucket and lines of every counted commit, in stream order, 16 B each: rework reads
    /// them twice.
    slots: Vec<(Option<usize>, Lines)>,
}

impl FrequencyTally {
    pub fn new(axis: &TimeAxis) -> Self {
        let out = axis
            .names()
            .map(|(key, label)| Frequency { key, label, additions: 0, deletions: 0, rewritten: 0 })
            .collect();
        Self { out, sum: LineTotals::default(), slots: Vec::new() }
    }

    /// The buckets, their totals (a commit outside the axis still counts in them) and the
    /// rework series over `c`'s axis.
    pub fn finish(self, c: &Counted<'_>) -> (BucketUnit, Vec<Frequency>, LineTotals, Rework) {
        let axis = c.axis();
        let counted = self.slots;
        let window = (counted.len() / 8).clamp(10, 100);
        // Prefix sums make every window O(1).
        let mut pre = vec![(0u64, 0u64, 0u64); counted.len() + 1];
        for (k, (_, l)) in counted.iter().enumerate() {
            let p = pre[k];
            pre[k + 1] =
                (p.0 + u64::from(l.add), p.1 + u64::from(l.del), p.2 + u64::from(l.rewritten));
        }
        // Counted commits up to and including each bucket.
        let mut upto = vec![0usize; axis.len()];
        for (k, (slot, _)) in counted.iter().enumerate() {
            if let Some(b) = slot.and_then(|b| upto.get_mut(b)) {
                *b = k + 1;
            }
        }
        let mut seen = 0;
        let points = axis
            .names()
            .zip(upto)
            .map(|((key, label), end)| {
                seen = seen.max(end);
                let start = seen.saturating_sub(window);
                let (a, d, w) = (
                    pre[seen].0 - pre[start].0,
                    pre[seen].1 - pre[start].1,
                    pre[seen].2 - pre[start].2,
                );
                ReworkPoint {
                    key,
                    label,
                    added: a,
                    removed: d,
                    rewritten: w,
                    share: rework_share(a, d, w),
                }
            })
            .collect();
        let rework = Rework { window, unit: axis.unit(), points };
        (axis.unit(), self.out, self.sum, rework)
    }
}

impl Reducer for FrequencyTally {
    fn add(&mut self, c: &Counted<'_>, item: &Item) {
        let slot = c.axis().slot_day(item.day);
        let l = item.lines;
        if let Some(w) = slot.and_then(|s| self.out.get_mut(s)) {
            w.additions += u64::from(l.add);
            w.deletions += u64::from(l.del);
            w.rewritten += u64::from(l.rewritten);
        }
        self.sum.additions += u64::from(l.add);
        self.sum.deletions += u64::from(l.del);
        self.sum.rewritten += u64::from(l.rewritten);
        self.slots.push((slot, l));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{ReworkPoint, rework_share, rework_verdict};
    use crate::insights::fixtures::{file, rec};
    use crate::{BucketUnit, FileStat, Insights, Options, Period};

    fn edit(path: &str, add: u32, del: u32, rewritten: u32) -> FileStat {
        FileStat { rewritten, ..file(path, add, del, 0) }
    }

    #[test]
    fn rework_math_and_verdicts() {
        assert_eq!(rework_share(0, 0, 0), None);
        assert_eq!(rework_share(10, 0, 0), Some(0.0));
        assert_eq!(rework_share(10, 10, 10), Some(1.0));
        assert_eq!(rework_verdict(0.1), "mostly new code");
        assert_eq!(rework_verdict(0.3), "growing while reworking");
        assert_eq!(rework_verdict(0.9), "mostly reworking existing code");
    }

    #[test]
    fn rolling_window_and_whitespace() {
        let mut noisy = edit("a.rs", 40, 40, 40);
        // Only 4 of those lines are real changes once whitespace is ignored.
        noisy.code = Some(crate::CodeStat { additions: 2, deletions: 2, rewritten: 2 });
        let recs = vec![
            rec("a", "Ann", 0, vec![edit("a.rs", 10, 0, 0)]),
            rec("b", "Ann", 1, vec![edit("a.rs", 5, 5, 5)]),
            rec("c", "Ann", 9, vec![noisy]),
        ];
        let ins = Insights::new(&recs, Options::default(), None);
        let rw = ins.rework(Period::All);
        let r = &rw.points;
        // Ten local days, a point each; the quiet days carry the window forward.
        assert_eq!((rw.window, r.len()), (10, 10));
        assert_eq!((r[1].added, r[1].removed, r[1].rewritten), (15, 5, 5));
        assert_eq!(r[1].share, Some(0.5));
        assert_eq!(
            r[5],
            ReworkPoint { key: r[5].key.clone(), label: r[5].label.clone(), ..r[1].clone() }
        );
        // Only 4 of c's 120 lines are real changes once whitespace is ignored.
        assert_eq!((r[9].added, r[9].removed, r[9].rewritten), (17, 7, 7));
        let ws =
            Insights::new(&recs, Options { count_whitespace: true, ..Options::default() }, None);
        assert_eq!(ws.rework(Period::All).points[9].added, 55);
        let (unit, f) = ins.frequency(Period::All);
        assert_eq!((unit, f.len()), (BucketUnit::Day, 10));
        assert_eq!((f[0].additions, f[0].deletions), (10, 0));
        assert_eq!((f[1].additions, f[1].deletions), (5, 5));
        let (_, week) = ins.frequency(Period::Week);
        assert_eq!(week.len(), 7);
    }

    #[test]
    fn rework_window_scales_with_the_selection() {
        let recs: Vec<_> =
            (0..400).map(|d| rec(&d.to_string(), "Ann", d, vec![edit("a.rs", 1, 0, 0)])).collect();
        let ins = Insights::new(&recs, Options::default(), None);
        assert_eq!(ins.rework(Period::All).window, 50);
        assert_eq!(ins.rework(Period::Week).window, 10);
        assert_eq!(ins.rework(Period::Week).points.len(), 7);
    }

    /// Week ending on day 20: local days 14..=20. Commit `e` sits on local day 13, one hour
    /// after the old UTC window opened, so the old window counted it and folded it into bar 0.
    pub(crate) fn edge_week() -> Vec<crate::CommitRecord> {
        let mut e = rec("e", "Ann", 13, vec![edit("a.rs", 1000, 0, 0)]);
        e.time += 3600;
        vec![
            e,
            rec("i", "Ann", 17, vec![edit("a.rs", 10, 0, 0)]),
            rec("n", "Bob", 20, vec![edit("a.rs", 5, 5, 5)]),
        ]
    }

    /// The frequency bars hold only the week's own local days.
    #[test]
    fn frequency_counts_only_the_local_days_of_the_period() {
        let recs = edge_week();
        let ins = Insights::new(&recs, Options::default(), None);
        let (_, f) = ins.frequency(Period::Week);
        assert_eq!(f.len(), 7);
        let add: u64 = f.iter().map(|b| b.additions).sum();
        assert_eq!(add, 15, "commit on local day 13 counted in a week of days 14..=20");
        assert_eq!(f[0].additions, 0);
    }

    #[test]
    fn rework_counts_only_the_local_days_of_the_period() {
        let recs = edge_week();
        let ins = Insights::new(&recs, Options::default(), None);
        let rw = ins.rework(Period::Week);
        let last = rw.points.last().expect("seven points");
        assert_eq!((last.added, last.removed), (15, 5));
    }

    /// The lines beside the frequency bars are the period's lines, read from their own
    /// reducer: the same as the totals of the period, never a sum over buckets.
    #[test]
    fn frequency_totals_are_the_period_lines() {
        let recs = edge_week();
        let ins = Insights::new(&recs, Options::default(), None);
        for p in [Period::Week, Period::Month, Period::All] {
            let (sum, totals) = ins.in_period(p, |c| (c.frequency_totals().2, c.totals()));
            assert_eq!(
                (sum.additions, sum.deletions),
                (totals.additions, totals.deletions),
                "{p:?}"
            );
        }
    }
}
