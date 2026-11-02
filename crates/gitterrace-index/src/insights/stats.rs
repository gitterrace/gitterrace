//! Churn per file and directory and conventional-commit label counts. Like every statistic
//! here they count non-merge commits by people who count under the options, over the files the
//! path filter keeps, inside the period's window.

use std::collections::HashMap;

use crate::feed::{CommitRange, CommitRow, FileRow, LabelId, PathId, Rank, Stream};
use crate::insights::fasthash::FastMap;
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

    /// The label id of a commit's columns, `None` when it has none.
    const fn id(self, c: &CommitRow) -> Option<LabelId> {
        match self {
            Self::Kind => c.kind,
            Self::Scope => c.scope,
        }
    }

    /// The name a commit without this label counts under.
    const fn missing(self) -> &'static str {
        match self {
            Self::Kind => Selection::OTHER,
            Self::Scope => Selection::NO_SCOPE,
        }
    }
}

impl Insights<'_> {
    /// Columns of the counted commits inside the period's window, oldest first.
    pub fn counted_rows(&self, period: Period) -> impl Iterator<Item = CommitRow> + '_ {
        let (from, to) = period.window(self.now);
        self.source()
            .commits_in(CommitRange::Time(from..to))
            .filter(move |c| self.counted_row(c))
    }

    /// File rows of the counted commits in the period, streamed, oldest first.
    fn counted_files(&self, period: Period) -> Stream<'_, (Rank, Vec<FileRow>)> {
        let ranks: Vec<Rank> = self.counted_rows(period).map(|c| c.rank).collect();
        self.source().rows_for(ranks)
    }

    /// Every counted file with its churn, most commits first, then path.
    pub fn file_churn(&self, period: Period) -> Vec<FileChurn> {
        struct Acc {
            /// Last commit counted, so a commit counts once per file in O(1).
            stamp: Rank,
            time: i64,
            commits: usize,
            additions: u64,
            deletions: u64,
        }
        let src = self.source();
        let mut map: FastMap<PathId, Acc> = FastMap::default();
        for (rank, rows) in self.counted_files(period) {
            let Some(time) = src.commit(rank).map(|c| c.time) else {
                continue;
            };
            for f in &rows {
                let Some(l) = self.row_lines(f) else {
                    continue;
                };
                let a = map.entry(f.path).or_insert(Acc {
                    stamp: Rank(u32::MAX),
                    time,
                    commits: 0,
                    additions: 0,
                    deletions: 0,
                });
                if a.stamp != rank {
                    a.stamp = rank;
                    a.commits += 1;
                }
                a.additions += u64::from(l.add);
                a.deletions += u64::from(l.del);
                a.time = a.time.max(time);
            }
        }
        let mut out: Vec<FileChurn> = map
            .into_iter()
            .map(|(id, a)| FileChurn {
                path: src.path(id).map(|p| p.to_string()).unwrap_or_default(),
                commits: a.commits,
                additions: a.additions,
                deletions: a.deletions,
                last_commit: a.time,
            })
            .collect();
        out.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.path.cmp(&b.path)));
        out
    }

    /// Churn per containing directory, most commits first, then path.
    pub fn dir_churn(&self, period: Period) -> Vec<DirChurn> {
        struct Acc {
            stamp: Rank,
            churn: DirChurn,
        }
        let src = self.source();
        let mut dirs: Vec<Acc> = Vec::new();
        let mut dir_slot: FastMap<String, usize> = FastMap::default();
        // Directory slot per path id, resolved once. A path belongs to one directory, so the
        // first sight of a path is a new file of its directory.
        let mut path_dir: FastMap<PathId, usize> = FastMap::default();
        for (rank, rows) in self.counted_files(period) {
            for f in &rows {
                let Some(l) = self.row_lines(f) else {
                    continue;
                };
                #[expect(
                    clippy::option_if_let_else,
                    clippy::single_match_else,
                    reason = "the miss arm inserts into path_dir, which get() still borrows in a closure"
                )]
                let (slot, new_path) = match path_dir.get(&f.path) {
                    Some(s) => (*s, false),
                    None => {
                        let path = src.path(f.path).unwrap_or_default();
                        let dir = path.rsplit_once('/').map_or(".", |(d, _)| d);
                        let s = *dir_slot.entry(dir.to_owned()).or_insert_with(|| {
                            dirs.push(Acc {
                                stamp: Rank(u32::MAX),
                                churn: DirChurn {
                                    path: dir.to_owned(),
                                    commits: 0,
                                    files: 0,
                                    additions: 0,
                                    deletions: 0,
                                },
                            });
                            dirs.len() - 1
                        });
                        path_dir.insert(f.path, s);
                        (s, true)
                    }
                };
                let a = &mut dirs[slot];
                if a.stamp != rank {
                    a.stamp = rank;
                    a.churn.commits += 1;
                }
                if new_path {
                    a.churn.files += 1;
                }
                a.churn.additions += u64::from(l.add);
                a.churn.deletions += u64::from(l.del);
            }
        }
        let mut out: Vec<DirChurn> = dirs.into_iter().map(|a| a.churn).collect();
        out.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.path.cmp(&b.path)));
        out
    }

    /// Commits per type or scope, most first.
    pub fn label_breakdown(&self, label: Label, period: Period) -> Vec<LabelCount> {
        let mut map: HashMap<Option<LabelId>, usize> = HashMap::new();
        for c in self.counted_rows(period) {
            *map.entry(label.id(&c)).or_default() += 1;
        }
        self.sorted_counts(label, map)
    }

    /// Type or scope counts per bucket of the period, empty buckets included.
    pub fn label_over_time(&self, label: Label, period: Period) -> Vec<LabelBucket> {
        let axis = self.axis(Some(period));
        let mut maps: Vec<HashMap<Option<LabelId>, usize>> = vec![HashMap::new(); axis.len()];
        for c in self.counted_rows(period) {
            if let Some(i) = axis.slot_row(&c) {
                *maps[i].entry(label.id(&c)).or_default() += 1;
            }
        }
        axis.names()
            .zip(maps)
            .map(|((key, name), counts)| LabelBucket {
                key,
                label: name,
                counts: self.sorted_counts(label, counts),
            })
            .collect()
    }

    /// Label counts by name, most first. Ids resolve to names first: the sort is by name.
    fn sorted_counts(&self, label: Label, map: HashMap<Option<LabelId>, usize>) -> Vec<LabelCount> {
        let src = self.source();
        let mut named: HashMap<String, usize> = HashMap::new();
        for (id, n) in map {
            let name = id
                .and_then(|i| src.label(i))
                .map_or_else(|| label.missing().to_owned(), |s| s.to_string());
            *named.entry(name).or_default() += n;
        }
        sorted_counts(named.iter().map(|(k, v)| (k.as_str(), *v)).collect())
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
