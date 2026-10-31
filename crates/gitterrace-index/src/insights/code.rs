use std::collections::{BTreeMap, HashMap};

use super::{Insights, bucket_label, bucket_start, local_day, next_bucket};
use crate::Granularity;

/// Weekly additions and deletions, moves and (by default) whitespace excluded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CodeFrequency {
    pub key: String,
    pub additions: u64,
    pub deletions: u64,
    pub rewritten: u64,
}

/// Rework over a rolling window of commits, sampled at the last commit of each week.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ReworkPoint {
    pub key: String,
    pub added: u64,
    pub removed: u64,
    pub rewritten: u64,
    /// Share of changed lines that replaced existing code. `None` when nothing changed.
    pub share: Option<f64>,
}

/// Lines of code per language at the end of each week.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LocSeries {
    /// Largest first at the end of history; ends with `Other` when languages were folded.
    pub languages: Vec<String>,
    /// Languages folded into `Other`, largest first.
    pub other: Vec<String>,
    pub points: Vec<LocPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LocPoint {
    pub key: String,
    /// Aligned with [`LocSeries::languages`].
    pub lines: Vec<i64>,
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
    pub fn code_frequency(&self) -> Vec<CodeFrequency> {
        let mut weeks: BTreeMap<i64, CodeFrequency> = BTreeMap::new();
        for (i, r) in self.records.iter().enumerate() {
            if !self.counted(i) {
                continue;
            }
            let l = self.commit_lines(i);
            let start = bucket_start(local_day(r), Granularity::Week);
            let w = weeks.entry(start).or_insert_with(|| CodeFrequency {
                key: bucket_label(start, Granularity::Week),
                additions: 0,
                deletions: 0,
                rewritten: 0,
            });
            w.additions += u64::from(l.add);
            w.deletions += u64::from(l.del);
            w.rewritten += u64::from(l.rewritten);
        }
        super::fill(weeks, Granularity::Week, |key| CodeFrequency {
            key,
            additions: 0,
            deletions: 0,
            rewritten: 0,
        })
    }

    /// Rolling rework over the last `window` counted commits.
    pub fn rework(&self, window: usize) -> Vec<ReworkPoint> {
        let window = window.max(1);
        let counted: Vec<usize> = (0..self.records.len())
            .filter(|i| self.counted(*i))
            .collect();
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
        let mut out: Vec<ReworkPoint> = Vec::new();
        for (k, i) in counted.iter().enumerate() {
            let week = bucket_start(local_day(&self.records[*i]), Granularity::Week);
            let end_of_week = counted.get(k + 1).is_none_or(|n| {
                bucket_start(local_day(&self.records[*n]), Granularity::Week) != week
            });
            if !end_of_week {
                continue;
            }
            let from = (k + 1).saturating_sub(window);
            let (a, d, w) = (
                pre[k + 1].0 - pre[from].0,
                pre[k + 1].1 - pre[from].1,
                pre[k + 1].2 - pre[from].2,
            );
            out.push(ReworkPoint {
                key: bucket_label(week, Granularity::Week),
                added: a,
                removed: d,
                rewritten: w,
                share: rework_share(a, d, w),
            });
        }
        out
    }

    /// Lines of code per language over time, exact: a running sum of additions minus deletions
    /// along the first-parent chain from the newest commit. Merge diffs against their first
    /// parent are how a branch's lines arrive, so merges count here. Whitespace is always
    /// counted so the final column equals the tree. `top` languages are kept, the rest folded.
    pub fn loc_over_time(&self, top: usize) -> LocSeries {
        let by_id: HashMap<&str, usize> = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.as_str(), i))
            .collect();
        let mut chain = Vec::new();
        let mut at = self.records.len().checked_sub(1);
        while let Some(i) = at {
            chain.push(i);
            at = self.records[i]
                .parents
                .first()
                .and_then(|p| by_id.get(p.as_str()).copied());
        }
        chain.reverse();

        let mut names: Vec<String> = Vec::new();
        let mut slot: HashMap<String, usize> = HashMap::new();
        let mut lang_of: HashMap<&str, usize> = HashMap::new();
        let mut running: Vec<i64> = Vec::new();
        let mut weeks: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        for i in chain {
            let r = &self.records[i];
            for f in &r.files {
                if self.filter.skips(&f.path) {
                    continue;
                }
                let s = *lang_of.entry(f.path.as_str()).or_insert_with(|| {
                    let l = super::language(&f.path);
                    *slot.entry(l.clone()).or_insert_with(|| {
                        names.push(l);
                        running.push(0);
                        names.len() - 1
                    })
                });
                running[s] += i64::from(f.additions) - i64::from(f.deletions);
            }
            weeks.insert(
                bucket_start(local_day(r), Granularity::Week),
                running.clone(),
            );
        }
        let final_row: Vec<i64> = running.clone();
        let mut order: Vec<usize> = (0..names.len()).filter(|s| final_row[*s] > 0).collect();
        order.sort_by(|a, b| {
            final_row[*b]
                .cmp(&final_row[*a])
                .then_with(|| names[*a].cmp(&names[*b]))
        });
        let keep: Vec<usize> = if order.len() <= top + 1 {
            order.clone()
        } else {
            order[..top].to_vec()
        };
        let folded: Vec<usize> = (0..names.len()).filter(|s| !keep.contains(s)).collect();
        let mut other: Vec<usize> = folded.clone();
        other.sort_by(|a, b| {
            final_row[*b]
                .cmp(&final_row[*a])
                .then_with(|| names[*a].cmp(&names[*b]))
        });
        let mut languages: Vec<String> = keep.iter().map(|s| names[*s].clone()).collect();
        if !folded.is_empty() {
            languages.push("Other".to_owned());
        }
        let mut points: Vec<LocPoint> = Vec::new();
        let mut carry = vec![0i64; names.len()];
        let (Some(first), Some(last)) = (
            weeks.keys().next().copied(),
            weeks.keys().next_back().copied(),
        ) else {
            return LocSeries {
                languages,
                other: Vec::new(),
                points,
            };
        };
        let mut at = first;
        while at <= last {
            if let Some(row) = weeks.get(&at) {
                carry.clone_from(row);
                carry.resize(names.len(), 0);
            }
            let mut lines: Vec<i64> = keep.iter().map(|s| carry[*s]).collect();
            if !folded.is_empty() {
                lines.push(folded.iter().map(|s| carry[*s]).sum());
            }
            points.push(LocPoint {
                key: bucket_label(at, Granularity::Week),
                lines,
            });
            at = next_bucket(at, Granularity::Week);
        }
        LocSeries {
            languages,
            other: other.iter().map(|s| names[*s].clone()).collect(),
            points,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{rework_share, rework_verdict};
    use crate::insights::fixtures::{file, rec};
    use crate::{FileStat, Insights, Options};

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
        let r = ins.rework(2);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].added, r[0].removed, r[0].rewritten), (15, 5, 5));
        assert_eq!(r[0].share, Some(0.5));
        // Window of 2 commits at the second week: b and c(ignoring whitespace).
        assert_eq!((r[1].added, r[1].removed, r[1].rewritten), (7, 7, 7));
        let ws = Insights::new(
            &recs,
            Options {
                count_whitespace: true,
                ..Options::default()
            },
            None,
        );
        assert_eq!(ws.rework(1)[1].added, 40);
        let f = ins.code_frequency();
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].additions, f[0].deletions), (15, 5));
    }

    #[test]
    fn loc_follows_first_parent_and_folds() {
        let mut a = rec(
            "a",
            "Ann",
            0,
            vec![file("x.rs", 100, 0, 100), file("y.py", 10, 0, 10)],
        );
        a.parents = vec![];
        let mut b = rec("b", "Ann", 1, vec![file("x.rs", 5, 25, 80)]);
        b.parents = vec!["a".into()];
        // A side branch commit that is not on the first-parent chain of the merge.
        let mut side = rec("s", "Bob", 2, vec![file("z.rs", 999, 0, 999)]);
        side.parents = vec!["a".into()];
        let mut m = rec("m", "Ann", 3, vec![file("w.md", 7, 0, 7)]);
        m.parents = vec!["b".into(), "s".into()];
        m.merge = true;
        let recs = vec![a, side, b, m];
        let ins = Insights::new(&recs, Options::default(), None);
        let loc = ins.loc_over_time(1);
        assert_eq!(loc.languages, ["Rust", "Other"]);
        assert_eq!(loc.other, ["Python", "Markdown"]);
        let last = loc.points.last().map(|p| p.lines.clone());
        assert_eq!(last, Some(vec![80, 17]));
        assert_eq!(loc.points.len(), 1);
    }
}
