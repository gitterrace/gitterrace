//! Churn per file and directory and conventional-commit label counts. Like every statistic
//! here they count non-merge commits by people who count under the options, over the files the
//! path filter keeps, inside the period's window.

use std::collections::HashMap;

use crate::insights::fasthash::{FastMap, FastSet};
use crate::insights::{Insights, Period, Selection};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileChurn {
    pub path: String,
    pub commits: usize,
    pub additions: u64,
    pub deletions: u64,
    pub last_commit: i64,
}

/// Churn of one directory, `.` for files at the repository root.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DirChurn {
    pub path: String,
    pub commits: usize,
    pub files: usize,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LabelCount {
    pub label: String,
    pub commits: usize,
}

/// Per-bucket counts of one label axis (type or scope).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LabelBucket {
    pub key: String,
    pub label: String,
    pub counts: Vec<LabelCount>,
}

/// Which conventional-commit field a breakdown groups by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Label {
    Kind,
    Scope,
}

impl Label {
    /// The label of a commit; commits without one count as [`Selection::OTHER`] (type) or
    /// [`Selection::NO_SCOPE`] (scope), the values the type and scope filters select them by.
    pub fn of(self, r: &crate::CommitRecord) -> &str {
        match self {
            Self::Kind => r.kind.as_deref().unwrap_or(Selection::OTHER),
            Self::Scope => r.scope.as_deref().unwrap_or(Selection::NO_SCOPE),
        }
    }
}

impl Insights<'_> {
    /// Indexes of the counted commits inside the period's window, oldest first.
    pub(crate) fn counted_in(&self, period: Period) -> impl Iterator<Item = usize> + '_ {
        let (from, to) = period.window(self.now);
        (0..self.records.len()).filter(move |i| {
            let t = self.records[*i].time;
            t >= from && t < to && self.counted(*i)
        })
    }

    /// Commits the statistics count in the period, oldest first.
    pub fn counted_records(&self, period: Period) -> impl Iterator<Item = &crate::CommitRecord> {
        self.counted_in(period).map(|i| &self.records[i])
    }

    /// Every counted file with its churn, most commits first, then path.
    pub fn file_churn(&self, period: Period) -> Vec<FileChurn> {
        struct Acc {
            /// Last commit counted, so a commit counts once per file in O(1).
            stamp: usize,
            churn: FileChurn,
        }
        let mut map: FastMap<&str, Acc> = FastMap::default();
        for i in self.counted_in(period) {
            let time = self.records[i].time;
            for (f, l) in self.files_of(i) {
                let a = map.entry(&*f.path).or_insert_with(|| Acc {
                    stamp: usize::MAX,
                    churn: FileChurn {
                        path: f.path.to_string(),
                        commits: 0,
                        additions: 0,
                        deletions: 0,
                        last_commit: time,
                    },
                });
                if a.stamp != i {
                    a.stamp = i;
                    a.churn.commits += 1;
                }
                a.churn.additions += u64::from(l.add);
                a.churn.deletions += u64::from(l.del);
                a.churn.last_commit = a.churn.last_commit.max(time);
            }
        }
        let mut out: Vec<FileChurn> = map.into_values().map(|a| a.churn).collect();
        out.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.path.cmp(&b.path)));
        out
    }

    /// Churn per containing directory, most commits first, then path.
    pub fn dir_churn(&self, period: Period) -> Vec<DirChurn> {
        struct Acc {
            stamp: usize,
            churn: DirChurn,
        }
        let mut map: FastMap<&str, Acc> = FastMap::default();
        // A path belongs to one directory, so first sight of a path is a new file of its directory.
        let mut paths: FastSet<&str> = FastSet::default();
        for i in self.counted_in(period) {
            for (f, l) in self.files_of(i) {
                let dir = f.path.rsplit_once('/').map_or(".", |(d, _)| d);
                let a = map.entry(dir).or_insert_with(|| Acc {
                    stamp: usize::MAX,
                    churn: DirChurn {
                        path: dir.to_owned(),
                        commits: 0,
                        files: 0,
                        additions: 0,
                        deletions: 0,
                    },
                });
                if a.stamp != i {
                    a.stamp = i;
                    a.churn.commits += 1;
                }
                if paths.insert(&*f.path) {
                    a.churn.files += 1;
                }
                a.churn.additions += u64::from(l.add);
                a.churn.deletions += u64::from(l.del);
            }
        }
        let mut out: Vec<DirChurn> = map.into_values().map(|a| a.churn).collect();
        out.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.path.cmp(&b.path)));
        out
    }

    /// Commits per type or scope, most first.
    pub fn label_breakdown(&self, label: Label, period: Period) -> Vec<LabelCount> {
        let mut map: HashMap<&str, usize> = HashMap::new();
        for r in self.counted_records(period) {
            *map.entry(label.of(r)).or_default() += 1;
        }
        sorted_counts(map)
    }

    /// Type or scope counts per bucket of the period, empty buckets included.
    pub fn label_over_time(&self, label: Label, period: Period) -> Vec<LabelBucket> {
        let axis = self.axis(Some(period));
        let mut maps: Vec<HashMap<&str, usize>> = vec![HashMap::new(); axis.len()];
        for r in self.counted_records(period) {
            if let Some(i) = axis.slot(r) {
                *maps[i].entry(label.of(r)).or_default() += 1;
            }
        }
        axis.names()
            .zip(maps)
            .map(|((key, label), counts)| LabelBucket {
                key,
                label,
                counts: sorted_counts(counts),
            })
            .collect()
    }
}

fn sorted_counts(map: HashMap<&str, usize>) -> Vec<LabelCount> {
    let mut out: Vec<LabelCount> = map
        .into_iter()
        .map(|(label, commits)| LabelCount {
            label: label.to_owned(),
            commits,
        })
        .collect();
    out.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.label.cmp(&b.label))
    });
    out
}

#[cfg(test)]
mod tests {
    use crate::insights::fixtures::{file, rec};
    use crate::{CommitRecord, FileStat, Insights, Label, Options, Period};

    fn typed(mut r: CommitRecord, kind: &str, scope: Option<&str>) -> CommitRecord {
        r.kind = Some(kind.into());
        r.scope = scope.map(Into::into);
        r
    }

    fn sample() -> Vec<CommitRecord> {
        vec![
            rec(
                "1",
                "Ann",
                0,
                vec![file("src/a.rs", 10, 0, 10), file("README.md", 1, 0, 1)],
            ),
            rec("2", "Bob", 1, vec![file("src/a.rs", 3, 2, 11)]),
            rec(
                "3",
                "Ann",
                8,
                vec![file("src/b.rs", 5, 5, 5), file("src/a.rs", 1, 1, 11)],
            ),
        ]
    }

    #[test]
    fn file_and_dir_churn() {
        let r = sample();
        let ins = Insights::new(&r, Options::default(), None);
        let f = ins.file_churn(Period::All);
        assert_eq!(f[0].path, "src/a.rs");
        assert_eq!((f[0].commits, f[0].additions, f[0].deletions), (3, 14, 3));
        assert_eq!(f[0].last_commit, r[2].time);
        let d = ins.dir_churn(Period::All);
        assert_eq!(d[0].path, "src");
        assert_eq!((d[0].commits, d[0].files), (3, 2));
        assert_eq!(d[1].path, ".");
        // The week ending at the newest commit holds only commit 3.
        let w = ins.file_churn(Period::Week);
        assert_eq!(w.iter().map(|c| c.commits).sum::<usize>(), 2);
    }

    #[test]
    fn churn_follows_the_options() {
        let mut r = sample();
        r[0].files.push(file("Cargo.lock", 99, 0, 99));
        r[1].merge = true;
        let ins = Insights::new(&r, Options::default(), None);
        let f = ins.file_churn(Period::All);
        assert!(
            f.iter().all(|c| c.path != "Cargo.lock"),
            "generated left out"
        );
        assert_eq!(f[0].commits, 2, "the merge does not count");
        let all = Insights::new(
            &r,
            Options {
                include_generated: true,
                ..Options::default()
            },
            None,
        );
        assert!(
            all.file_churn(Period::All)
                .iter()
                .any(|c| c.path == "Cargo.lock")
        );
    }

    #[test]
    fn same_file_twice_in_one_commit_counts_one_commit() {
        let r = [rec(
            "1",
            "a",
            0,
            vec![file("x", 1, 0, 1), file("x", 2, 0, 3)],
        )];
        let ins = Insights::new(&r, Options::default(), None);
        assert_eq!(ins.file_churn(Period::All)[0].commits, 1);
    }

    /// One commit touching tens of thousands of files, as a vendoring or tree-wide rename does.
    /// The per-commit dedup must stay linear: a scan of the files already seen made this case
    /// quadratic and cost seconds on a large repository. The test is sized so that a quadratic
    /// version would run for minutes, and it also checks every file counts exactly once.
    #[test]
    fn a_commit_touching_very_many_files_stays_linear() {
        let n = 60_000;
        let files: Vec<FileStat> = (0..n)
            .flat_map(|i| {
                let path: std::sync::Arc<str> = format!("d{}/f{i}.rs", i % 50).into();
                // Each file listed twice in the commit: still one commit for it.
                (0..2).map(move |_| FileStat {
                    path: std::sync::Arc::clone(&path),
                    additions: 1,
                    ..FileStat::default()
                })
            })
            .collect();
        let r = vec![rec("big", "a", 0, files)];
        let ins = Insights::new(&r, Options::default(), None);
        let churn = ins.file_churn(Period::All);
        assert_eq!(churn.len(), n);
        assert!(churn.iter().all(|c| c.commits == 1 && c.additions == 2));
        let dirs = ins.dir_churn(Period::All);
        assert_eq!(dirs.len(), 50);
        assert!(dirs.iter().all(|d| d.commits == 1));
    }

    #[test]
    fn type_and_scope_breakdown() {
        let r = vec![
            typed(rec("1", "a", 0, vec![]), "feat", Some("web")),
            typed(rec("2", "a", 40, vec![]), "feat", None),
            typed(rec("3", "a", 41, vec![]), "fix", Some("web")),
            rec("4", "a", 0, vec![]),
        ];
        let ins = Insights::new(&r, Options::default(), None);
        let kinds = ins.label_breakdown(Label::Kind, Period::All);
        assert_eq!((kinds[0].label.as_str(), kinds[0].commits), ("feat", 2));
        // Commits without a type count as `other`, so the counts add up to the commits.
        assert_eq!(kinds.iter().map(|k| k.commits).sum::<usize>(), 4);
        let scopes = ins.label_breakdown(Label::Scope, Period::All);
        assert_eq!((scopes[0].label.as_str(), scopes[0].commits), ("none", 2));
        let over = ins.label_over_time(Label::Kind, Period::All);
        // Weekly buckets: week 0 holds feat and other; days 40 (Sat) and 41 (Sun) fall in one
        // week holding feat and fix.
        assert_eq!(over[0].key, "2024-01-01");
        assert_eq!(over[0].counts.len(), 2);
        assert_eq!(over.iter().map(|b| b.counts.len()).sum::<usize>(), 4);
    }
}
