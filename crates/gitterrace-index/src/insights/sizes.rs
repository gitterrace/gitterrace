//! Commits bucketed by size, the histogram behind the commit-size chart.

use crate::insights::{Insights, Period};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SizeBucket {
    /// `1`, `2-5`, `6-20`, `21-100`, `101-500`, `501-2000`, `2000+`.
    pub label: String,
    pub commits: usize,
}

#[expect(
    clippy::redundant_closure_for_method_calls,
    reason = "a method path binds one Counted lifetime, in_period needs a closure over any"
)]
impl Insights<'_> {
    /// Commits by size over the whole history, see [`Insights::commit_sizes_in`].
    pub fn commit_sizes(&self) -> Vec<SizeBucket> {
        self.commit_sizes_in(Period::All)
    }

    /// Commits of `period` by size (added plus removed lines over counted files).
    pub fn commit_sizes_in(&self, period: Period) -> Vec<SizeBucket> {
        self.in_period(period, |c| c.sizes())
    }
}

#[cfg(test)]
mod tests {
    use crate::insights::fixtures::chain;
    use crate::{Insights, Options, Period};

    #[test]
    fn sizes_bucket_by_changed_lines() {
        let recs = chain();
        let s = Insights::new(&recs, Options::default(), None).commit_sizes();
        let counts: Vec<usize> = s.iter().map(|b| b.commits).collect();
        assert_eq!(counts, [1, 0, 0, 1, 1, 0, 1]);
    }

    /// The histogram follows the period. The chain's newest commit is day 40, so the week
    /// holds only it (3000 lines, the last bucket).
    #[test]
    fn sizes_follow_the_period() {
        let recs = chain();
        let s = Insights::new(&recs, Options::default(), None).commit_sizes_in(Period::Week);
        let counts: Vec<usize> = s.iter().map(|b| b.commits).collect();
        assert_eq!(counts, [0, 0, 0, 0, 0, 0, 1]);
    }
}
