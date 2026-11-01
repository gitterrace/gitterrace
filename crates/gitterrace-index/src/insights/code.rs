use crate::insights::{Insights, Period};

/// Additions and deletions of one bucket, moves and (by default) whitespace excluded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CodeFrequency {
    pub key: String,
    pub label: String,
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
    /// Commits in the rolling window, scaled to the selection.
    pub window: usize,
    pub unit: super::BucketUnit,
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

impl Insights<'_> {
    pub fn code_frequency(&self, period: Period) -> (super::BucketUnit, Vec<CodeFrequency>) {
        let (from, to) = period.window(self.now);
        let axis = self.axis(Some(period));
        let mut out: Vec<CodeFrequency> = axis
            .names()
            .map(|(key, label)| CodeFrequency {
                key,
                label,
                additions: 0,
                deletions: 0,
                rewritten: 0,
            })
            .collect();
        for (i, r) in self.records.iter().enumerate() {
            if r.time < from || r.time >= to || !self.counted(i) {
                continue;
            }
            let Some(w) = axis.slot(r).map(|s| &mut out[s]) else {
                continue;
            };
            let l = self.commit_lines(i);
            w.additions += u64::from(l.add);
            w.deletions += u64::from(l.del);
            w.rewritten += u64::from(l.rewritten);
        }
        (axis.unit(), out)
    }

    /// Rolling rework read at the end of every bucket of `period`. The window is an eighth of
    /// the commits in the selection, between 10 and 100: a week of a dozen commits must not
    /// average over a hundred older ones, and a big history needs the 100 to smooth out.
    pub fn rework(&self, period: Period) -> Rework {
        let (from, to) = period.window(self.now);
        let axis = self.axis(Some(period));
        let counted: Vec<usize> = (0..self.records.len())
            .filter(|i| self.counted(*i) && (from..to).contains(&self.records[*i].time))
            .collect();
        let window = (counted.len() / 8).clamp(10, 100);
        let lines: Vec<super::Lines> = counted.iter().map(|i| self.commit_lines(*i)).collect();
        // Prefix sums make every window O(1).
        let mut pre = vec![(0u64, 0u64, 0u64); lines.len() + 1];
        for (k, l) in lines.iter().enumerate() {
            let p = pre[k];
            pre[k + 1] = (
                p.0 + u64::from(l.add),
                p.1 + u64::from(l.del),
                p.2 + u64::from(l.rewritten),
            );
        }
        // Counted commits up to and including each bucket.
        let mut upto = vec![0usize; axis.len()];
        for (k, i) in counted.iter().enumerate() {
            if let Some(b) = axis.slot(&self.records[*i]) {
                upto[b] = k + 1;
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
        Rework {
            window,
            unit: axis.unit(),
            points,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ReworkPoint, rework_share, rework_verdict};
    use crate::insights::fixtures::{file, rec};
    use crate::{BucketUnit, FileStat, Insights, Options, Period};

    fn edit(path: &str, add: u32, del: u32, rewritten: u32) -> FileStat {
        FileStat {
            rewritten,
            ..file(path, add, del, 0)
        }
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
        noisy.code = Some(crate::CodeStat {
            additions: 2,
            deletions: 2,
            rewritten: 2,
        });
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
            ReworkPoint {
                key: r[5].key.clone(),
                label: r[5].label.clone(),
                ..r[1].clone()
            }
        );
        // Only 4 of c's 120 lines are real changes once whitespace is ignored.
        assert_eq!((r[9].added, r[9].removed, r[9].rewritten), (17, 7, 7));
        let ws = Insights::new(
            &recs,
            Options {
                count_whitespace: true,
                ..Options::default()
            },
            None,
        );
        assert_eq!(ws.rework(Period::All).points[9].added, 55);
        let (unit, f) = ins.code_frequency(Period::All);
        assert_eq!((unit, f.len()), (BucketUnit::Day, 10));
        assert_eq!((f[0].additions, f[0].deletions), (10, 0));
        assert_eq!((f[1].additions, f[1].deletions), (5, 5));
        let (_, week) = ins.code_frequency(Period::Week);
        assert_eq!(week.len(), 7);
    }

    #[test]
    fn rework_window_scales_with_the_selection() {
        let recs: Vec<_> = (0..400)
            .map(|d| rec(&d.to_string(), "Ann", d, vec![edit("a.rs", 1, 0, 0)]))
            .collect();
        let ins = Insights::new(&recs, Options::default(), None);
        assert_eq!(ins.rework(Period::All).window, 50);
        assert_eq!(ins.rework(Period::Week).window, 10);
        assert_eq!(ins.rework(Period::Week).points.len(), 7);
    }
}
