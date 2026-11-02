//! Commits bucketed by size, the histogram behind the commit-size chart.

use crate::feed::CommitRange;
use crate::insights::Insights;

/// Upper bounds (inclusive) of the commit size buckets, in changed lines.
const SIZE_EDGES: [u32; 6] = [1, 5, 20, 100, 500, 2000];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SizeBucket {
    /// `1`, `2-5`, `6-20`, `21-100`, `101-500`, `501-2000`, `2000+`.
    pub label: String,
    pub commits: usize,
}

impl Insights<'_> {
    /// Commits by size (added plus removed lines over counted files).
    pub fn commit_sizes(&self) -> Vec<SizeBucket> {
        let mut counts = [0usize; 7];
        for c in self.source().commits_in(CommitRange::All) {
            if !self.counted_row(&c) {
                continue;
            }
            let l = self.commit_sums(c.rank);
            let n = l.add + l.del;
            let slot = SIZE_EDGES
                .iter()
                .position(|e| n <= *e)
                .unwrap_or(SIZE_EDGES.len());
            counts[slot] += 1;
        }
        let labels = ["1", "2-5", "6-20", "21-100", "101-500", "501-2000", "2000+"];
        labels
            .iter()
            .zip(counts)
            .map(|(l, commits)| SizeBucket {
                label: (*l).to_owned(),
                commits,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::insights::fixtures::chain;
    use crate::{Insights, Options};

    #[test]
    fn sizes_bucket_by_changed_lines() {
        let recs = chain();
        let s = Insights::new(&recs, Options::default(), None).commit_sizes();
        let counts: Vec<usize> = s.iter().map(|b| b.commits).collect();
        assert_eq!(counts, [1, 0, 0, 1, 1, 0, 1]);
    }
}
