use std::collections::{HashMap, HashSet};

use gitterrace_git::{RefInfo, RefKind};

use crate::insights::{Insights, language};

/// Upper bounds (inclusive) of the commit size buckets, in changed lines.
const SIZE_EDGES: [u32; 6] = [1, 5, 20, 100, 500, 2000];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SizeBucket {
    /// `1`, `2-5`, `6-20`, `21-100`, `101-500`, `501-2000`, `2000+`.
    pub label: String,
    pub commits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Release {
    pub name: String,
    pub commit: String,
    pub time: i64,
    /// Commits since the previous release (first release: all commits up to it).
    pub commits_since: usize,
    pub days_since: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Cadence {
    /// Oldest first.
    pub releases: Vec<Release>,
    pub median_days_between: Option<f64>,
    /// Days from the newest release to the newest commit.
    pub days_since_last: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LangDelta {
    pub language: String,
    pub added: u64,
    pub removed: u64,
    pub net: i64,
    pub rewritten: u64,
}

/// `a..b`: commits reachable from `b` but not from `a`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Comparison {
    pub commits: Vec<String>,
    pub authors: usize,
    pub files_changed: usize,
    pub added: u64,
    pub removed: u64,
    /// Largest net change first.
    pub languages: Vec<LangDelta>,
}

/// `(name, commit id)` of every tag, as [`Insights::cadence`] takes them.
pub fn release_tags(refs: &[RefInfo]) -> Vec<(String, String)> {
    refs.iter()
        .filter(|r| r.kind == RefKind::Tag)
        .map(|r| (r.name.clone(), r.target.clone()))
        .collect()
}

impl Insights<'_> {
    /// Commits by size (added plus removed lines over counted files).
    pub fn commit_sizes(&self) -> Vec<SizeBucket> {
        let mut counts = [0usize; 7];
        for i in (0..self.records.len()).filter(|i| self.counted(*i)) {
            let l = self.commit_lines(i);
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

    /// Release rhythm from tags. Tags whose commit is not in the records are dropped.
    pub fn cadence(&self, tags: &[(String, String)]) -> Cadence {
        let pos: HashMap<&str, usize> = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.as_str(), i))
            .collect();
        let mut found: Vec<(usize, &str)> = tags
            .iter()
            .filter_map(|(n, id)| pos.get(id.as_str()).map(|i| (*i, n.as_str())))
            .collect();
        found.sort_by_key(|(i, n)| (self.records[*i].time, *i, *n));
        let mut releases = Vec::new();
        let mut prev: Option<usize> = None;
        for (i, name) in found {
            let r = &self.records[i];
            releases.push(Release {
                name: name.to_owned(),
                commit: r.id.clone(),
                time: r.time,
                commits_since: prev.map_or(i + 1, |p| i.saturating_sub(p)),
                days_since: prev.map(|p| (r.time - self.records[p].time).div_euclid(86_400)),
            });
            prev = Some(i);
        }
        let mut gaps: Vec<i64> = releases.iter().filter_map(|r| r.days_since).collect();
        gaps.sort_unstable();
        let median = match gaps.len() {
            0 => None,
            n if n % 2 == 1 => Some(gaps[n / 2] as f64),
            n => Some((gaps[n / 2 - 1] + gaps[n / 2]) as f64 / 2.0),
        };
        Cadence {
            days_since_last: releases
                .last()
                .map(|r| (self.now - r.time).div_euclid(86_400)),
            median_days_between: median,
            releases,
        }
    }

    /// Per-language change between two revisions given as commit ids, from the cached
    /// non-merge commits in `a..b`. Unknown ids give an empty comparison.
    pub fn compare(&self, base_id: &str, head_id: &str) -> Comparison {
        let by_id: HashMap<&str, usize> = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.as_str(), i))
            .collect();
        let reach = |from: &str| -> HashSet<usize> {
            let mut seen = HashSet::new();
            let mut stack: Vec<usize> = by_id.get(from).copied().into_iter().collect();
            while let Some(i) = stack.pop() {
                if !seen.insert(i) {
                    continue;
                }
                stack.extend(
                    self.records[i]
                        .parents
                        .iter()
                        .filter_map(|p| by_id.get(p.as_str()).copied()),
                );
            }
            seen
        };
        let base = reach(base_id);
        let mut range: Vec<usize> = reach(head_id)
            .into_iter()
            .filter(|i| !base.contains(i))
            .collect();
        range.sort_unstable();
        let mut langs: HashMap<String, (u64, u64, u64)> = HashMap::new();
        let mut people = HashSet::new();
        let mut paths: HashSet<&str> = HashSet::new();
        let (mut added, mut removed) = (0, 0);
        for &i in &range {
            let rec = &self.records[i];
            if rec.merge {
                continue;
            }
            people.insert(self.person[i]);
            for file in &rec.files {
                let Some(l) = self.lines(file) else { continue };
                paths.insert(&*file.path);
                let e = langs.entry(language(&file.path)).or_default();
                e.0 += u64::from(l.add);
                e.1 += u64::from(l.del);
                e.2 += u64::from(l.rewritten);
                added += u64::from(l.add);
                removed += u64::from(l.del);
            }
        }
        let mut languages: Vec<LangDelta> = langs
            .into_iter()
            .map(|(language, (added, removed, rewritten))| LangDelta {
                language,
                added,
                removed,
                net: added as i64 - removed as i64,
                rewritten,
            })
            .collect();
        languages.sort_by(|x, y| {
            y.net
                .abs()
                .cmp(&x.net.abs())
                .then_with(|| x.language.cmp(&y.language))
        });
        Comparison {
            commits: range.iter().map(|i| self.records[*i].id.clone()).collect(),
            authors: people.len(),
            files_changed: paths.len(),
            added,
            removed,
            languages,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::insights::fixtures::{file, rec};
    use crate::{CommitRecord, Insights, Options};

    fn chain() -> Vec<CommitRecord> {
        let mut v = vec![
            rec("a", "Ann", 0, vec![file("x.rs", 1, 0, 1)]),
            rec("b", "Ann", 10, vec![file("x.rs", 30, 0, 31)]),
            rec(
                "c",
                "Bob",
                30,
                vec![file("y.py", 300, 100, 200), file("x.rs", 0, 3, 28)],
            ),
            rec("d", "Bob", 40, vec![file("y.py", 3000, 0, 3200)]),
        ];
        for i in 1..v.len() {
            v[i].parents = vec![v[i - 1].id.clone()];
        }
        v
    }

    #[test]
    fn sizes_bucket_by_changed_lines() {
        let recs = chain();
        let s = Insights::new(&recs, Options::default(), None).commit_sizes();
        let counts: Vec<usize> = s.iter().map(|b| b.commits).collect();
        assert_eq!(counts, [1, 0, 0, 1, 1, 0, 1]);
    }

    #[test]
    fn cadence_from_tags() {
        let recs = chain();
        let ins = Insights::new(&recs, Options::default(), None);
        let tags = vec![
            ("v2".to_owned(), "d".to_owned()),
            ("v1".to_owned(), "b".to_owned()),
            ("ghost".to_owned(), "zz".to_owned()),
        ];
        let c = ins.cadence(&tags);
        assert_eq!(c.releases.len(), 2);
        assert_eq!(c.releases[0].name, "v1");
        assert_eq!(c.releases[0].commits_since, 2);
        assert_eq!(c.releases[1].commits_since, 2);
        assert_eq!(c.releases[1].days_since, Some(30));
        assert_eq!(c.median_days_between, Some(30.0));
        assert_eq!(c.days_since_last, Some(0));
    }

    #[test]
    fn compare_sums_commits_in_range() {
        let recs = chain();
        let ins = Insights::new(&recs, Options::default(), None);
        let c = ins.compare("b", "d");
        assert_eq!(c.commits, ["c", "d"]);
        assert_eq!(c.authors, 1);
        assert_eq!(c.files_changed, 2);
        assert_eq!(c.languages[0].language, "Python");
        assert_eq!(c.languages[0].net, 3200);
        assert_eq!(c.languages[1].net, -3);
        assert!(ins.compare("b", "nope").commits.is_empty());
    }
}
