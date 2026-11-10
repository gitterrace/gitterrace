//! The per-commit line table every sum reads, and the all-time counts it holds besides.

use crate::feed::{CommitRange, Rank};
use crate::insights::{Insights, Lines, rank_of};

/// [`Insights::commit_sums`] per rank and the distinct counted files of the history, from
/// one read of every file row.
pub(super) struct CommitLines {
    lines: Vec<Lines>,
    files: usize,
}

impl Insights<'_> {
    /// [`Insights::commit_sums`] per rank, read from every file row once per set.
    pub(super) fn commit_lines(&self) -> &[Lines] {
        &self.line_table().lines
    }

    /// Distinct counted files of the whole history under these options: what the all-time
    /// totals count, read with the per-commit lines instead of by a second walk of every row.
    pub(crate) fn all_files(&self) -> usize {
        self.line_table().files
    }

    fn line_table(&self) -> &CommitLines {
        self.d.commit_lines.get_or_init(|| {
            let src = self.source();
            let all: Vec<Rank> = (0..src.len()).map(rank_of).collect();
            let mut cols = src.commits_in(CommitRange::All);
            let mut seen = vec![false; src.paths()];
            let mut files = 0;
            let lines = src
                .rows_for(all)
                .map(|(r, rows)| {
                    let col = cols.by_ref().find(|c| c.rank == r);
                    let counts = col.is_some_and(|c| !c.merge && self.counted_row(&c));
                    rows.iter()
                        .filter_map(|row| {
                            let l = self.row_lines(row)?;
                            if counts
                                && let Some(s) = seen.get_mut(row.path.0 as usize)
                                && !*s
                            {
                                *s = true;
                                files += 1;
                            }
                            Some(l)
                        })
                        .fold(Lines::default(), |mut out, l| {
                            out.add += l.add;
                            out.del += l.del;
                            out.rewritten += l.rewritten;
                            out
                        })
                })
                .collect();
            CommitLines { lines, files }
        })
    }
}
