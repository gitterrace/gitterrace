//! [`Figure`]: a count only the index builds, carrying the population it counts.
//!
//! A page or the API renders a `Figure`, never a bare number, so a count that did not come
//! from a reducer over one population does not type-check into a figure.

use crate::feed::CommitRange;
use crate::insights::{Insights, PeriodStats};
pub use gitterrace_types::api::Population;

/// A count and the population it covers. Built only here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Figure {
    value: u64,
    population: Population,
}

impl Figure {
    const fn new(value: u64, population: Population) -> Self {
        Self { value, population }
    }

    pub const fn value(self) -> u64 {
        self.value
    }

    pub const fn population(self) -> Population {
        self.population
    }

    /// The wire form, for the API and MCP.
    pub const fn item(self) -> gitterrace_types::api::FigureItem {
        gitterrace_types::api::FigureItem {
            population: self.population,
            value: self.value,
        }
    }
}

/// Every reachable commit, and how many of them are merges and how many bots wrote: the
/// counts of [`Population::Every`], from one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EveryTotals {
    pub commits: Figure,
    pub merges: Figure,
    pub bots: Figure,
}

impl PeriodStats {
    /// The commits of these totals as a [`Population::Counted`] figure.
    pub const fn commits_figure(&self) -> Figure {
        Figure::new(self.commits as u64, Population::Counted)
    }
}

impl Insights<'_> {
    /// [`EveryTotals`] over this whole history. Bots are counted by person, after mailmap,
    /// whatever the options say: the options decide what counts, not what exists.
    pub fn every_totals(&self) -> EveryTotals {
        let (mut all, mut merges, mut bots) = (0u64, 0u64, 0u64);
        for c in self.source().commits_in(CommitRange::All) {
            all += 1;
            merges += u64::from(c.merge);
            bots += u64::from(self.d.people.list[self.person_of(c.ident)].bot);
        }
        EveryTotals {
            commits: Figure::new(all, Population::Every),
            merges: Figure::new(merges, Population::Every),
            bots: Figure::new(bots, Population::Every),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::insights::fixtures::rec;
    use crate::{Insights, Options, Period, Population};

    /// Three commits by Ann, one merge, two by a bot: every commit is six, the counted ones
    /// three, and each figure says which it is.
    #[test]
    fn every_and_counted_name_their_population() {
        let mut recs = vec![
            rec("a", "Ann", 0, vec![]),
            rec("b", "Ann", 1, vec![]),
            rec("c", "Ann", 2, vec![]),
            rec("m", "Ann", 3, vec![]),
            rec("d1", "dependabot[bot]", 4, vec![]),
            rec("d2", "dependabot[bot]", 5, vec![]),
        ];
        recs[3].merge = true;
        let ins = Insights::new(&recs, Options::default(), None);
        let e = ins.every_totals();
        assert_eq!(
            (e.commits.value(), e.merges.value(), e.bots.value()),
            (6, 1, 2)
        );
        assert_eq!(e.commits.population(), Population::Every);
        let c = ins.period_summary(Period::All).current.commits_figure();
        assert_eq!((c.value(), c.population()), (3, Population::Counted));
    }
}
