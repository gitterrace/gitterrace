//! [`ActivityFold`]: every count the activity page and its sections show, from one pass.
//!
//! The page shows totals, contributors, two label tallies, directories, files, code frequency
//! and rework. Each is a reducer, and one [`Counted::reduce`] feeds them all: a walk per
//! reader costs a pass with rows each.

use crate::feed::CommitRow;
use crate::insights::period::ContributorTally;
use crate::insights::stats::{DirTally, FileTally};
use crate::insights::{
    BucketUnit, ContributorReport, Counted, DirChurn, FileChurn, Frequency, FrequencyTally, Item,
    Label, LabelBucket, LabelCount, LabelTally, LineTotals, Metric, PeriodSummary, Reducer, Rework,
    Rhythm, Series, Totals, When,
};

/// What [`Counted::activity_fold`] read, each field equal to its one-pass reader.
pub struct ActivityFold {
    /// [`Counted::activity`].
    pub summary: PeriodSummary,
    pub rhythm: Rhythm,
    /// [`Counted::stream`], oldest rank first.
    pub rows: Vec<CommitRow>,
    /// [`Counted::contributors`] by commits.
    pub people: ContributorReport,
    /// [`Counted::per_label`] of types and of scopes.
    pub kinds: (Vec<LabelCount>, Vec<LabelBucket>),
    pub scopes: (Vec<LabelCount>, Vec<LabelBucket>),
    pub dirs: Vec<DirChurn>,
    pub files: Vec<FileChurn>,
    /// [`Counted::frequency_totals`] and [`Counted::rework`].
    pub frequency: (BucketUnit, Vec<Frequency>, LineTotals),
    pub rework: Rework,
}

/// The commit of every item, in stream order.
struct Rows(Vec<CommitRow>);

impl Reducer for Rows {
    fn add(&mut self, _: &Counted<'_>, item: &Item) {
        self.0.push(item.commit);
    }
}

impl Counted<'_> {
    /// Every count of the activity page in one pass over the set (the "before" column and
    /// the first-time check of a fixed period read their own, earlier days).
    pub fn activity_fold(&self) -> ActivityFold {
        let axis = self.axis();
        let mut totals = Totals::default();
        let mut series = Series::new(axis);
        let mut when = When::default();
        let mut rows = Rows(Vec::new());
        let mut people = ContributorTally::new(Metric::Commits, axis);
        let mut kinds = LabelTally::new(Label::Kind, axis);
        let mut scopes = LabelTally::new(Label::Scope, axis);
        let mut dirs = DirTally::default();
        let mut files = FileTally::default();
        let mut freq = FrequencyTally::new(axis);
        self.reduce(&mut [
            &mut totals,
            &mut series,
            &mut when,
            &mut rows,
            &mut people,
            &mut kinds,
            &mut scopes,
            &mut dirs,
            &mut files,
            &mut freq,
        ]);
        let (summary, rhythm) = self.activity_of(totals, series, &when);
        let (unit, weeks, sum, rework) = freq.finish(self);
        ActivityFold {
            summary,
            rhythm,
            rows: rows.0,
            people: people.finish(self),
            kinds: kinds.finish(self),
            scopes: scopes.finish(self),
            dirs: dirs.finish(),
            files: files.finish(self),
            frequency: (unit, weeks, sum),
            rework,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::feed::{FixedClock, MemorySource};
    use crate::insights::fixtures::{MON, chain};
    use crate::insights::{CountContext, Insights, Label, Metric, Options, Period, Scope};
    use crate::insights::{ScopeParams, Selection};

    fn scope(period: &str, path: Option<&str>) -> Scope {
        let mut p = ScopeParams {
            period: Some(period.to_owned()),
            ..ScopeParams::default()
        };
        p.select = Selection {
            path: path.map(str::to_owned),
            ..Selection::default()
        };
        Scope::from_params(&p, Options::default(), Period::All).expect("scope")
    }

    /// Guard class: one page, one walk. Every field equals its own reader, and the fold
    /// streams the set once for all time (red at 9 when each reader walks on its own).
    #[test]
    fn the_activity_fold_reads_the_set_once_and_agrees_with_each_reader() {
        let recs = chain();
        let ins = Insights::new(&recs, Options::default(), None);
        let cx = CountContext::new(&MemorySource::new(&recs), &FixedClock(MON + 100 * 86_400));
        for s in [scope("all", None), scope("all", Some("x.rs"))] {
            let c = ins.counted(&s, cx);
            let before = ins.walks();
            let f = c.activity_fold();
            assert_eq!(
                ins.walks() - before,
                1,
                "walks of the counted set for one fold"
            );
            let (summary, rhythm) = c.activity();
            assert_eq!(f.summary, summary);
            assert_eq!(f.rhythm, rhythm);
            assert_eq!(f.rows, c.stream().collect::<Vec<_>>());
            assert_eq!(f.people, c.contributors(Metric::Commits));
            assert_eq!(f.kinds, c.per_label(Label::Kind));
            assert_eq!(f.scopes, c.per_label(Label::Scope));
            assert_eq!(f.dirs, c.per_dir());
            assert_eq!(f.files, c.per_file());
            assert_eq!(f.frequency, c.frequency_totals());
            assert_eq!(f.rework, c.rework());
        }
    }
}
