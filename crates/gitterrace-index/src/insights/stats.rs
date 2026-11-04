//! Churn per file and directory and conventional-commit label counts. Like every statistic
//! here they count non-merge commits by people who count under the options, over the files the
//! path filter keeps, inside the period's window.

use crate::feed::{CommitRow, FileRow, FixedClock, PathId, Rank};
use crate::insights::fasthash::FastMap;
use crate::insights::{CountContext, Counted, Insights, Period, Scope, ScopeParams, Selection};

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

#[expect(
    clippy::redundant_closure_for_method_calls,
    reason = "a method path binds one Counted lifetime, in_period needs a closure over any"
)]
impl Insights<'_> {
    /// `f` over the commits `period` counts in this whole history, under these options, with
    /// the context anchored on this history: what the period-keyed page functions count.
    /// `None` only if the scope were refused, which empty parameters never are.
    pub(crate) fn in_period<T>(
        &self,
        period: Period,
        f: impl FnOnce(&Counted<'_>) -> T,
    ) -> Option<T> {
        let s = Scope::from_params(&ScopeParams::default(), self.opts.clone(), period).ok()?;
        let cx = CountContext::new(self.source(), &FixedClock(self.now));
        Some(f(&self.counted(&s, cx)))
    }

    /// Columns of the counted commits of the period, oldest first.
    pub fn counted_rows(&self, period: Period) -> impl Iterator<Item = CommitRow> {
        self.in_period(period, |c| c.stream().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
    }

    /// Every counted file with its churn, most commits first, then path.
    pub fn file_churn(&self, period: Period) -> Vec<FileChurn> {
        self.in_period(period, |c| c.per_file()).unwrap_or_default()
    }

    /// Churn per containing directory, most commits first, then path.
    pub fn dir_churn(&self, period: Period) -> Vec<DirChurn> {
        self.in_period(period, |c| c.per_dir()).unwrap_or_default()
    }

    /// Commits per type or scope, most first.
    pub fn label_breakdown(&self, label: Label, period: Period) -> Vec<LabelCount> {
        self.in_period(period, |c| c.per_label(label).0)
            .unwrap_or_default()
    }

    /// Type or scope counts per bucket of the period, empty buckets included.
    pub fn label_over_time(&self, label: Label, period: Period) -> Vec<LabelBucket> {
        self.in_period(period, |c| c.per_label(label).1)
            .unwrap_or_default()
    }
}

impl Counted<'_> {
    /// Every counted file with its churn, most commits first, then path.
    pub fn per_file(&self) -> Vec<FileChurn> {
        struct Acc {
            /// Last commit counted, so a commit counts once per file in O(1).
            stamp: Rank,
            time: i64,
            commits: usize,
            additions: u64,
            deletions: u64,
        }
        let ins = self.insights();
        let src = ins.source();
        let mut map: FastMap<PathId, Acc> = FastMap::default();
        for (rank, time, rows) in self.commit_rows() {
            for f in &rows {
                let Some(l) = ins.row_lines(f) else {
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
    pub fn per_dir(&self) -> Vec<DirChurn> {
        struct Acc {
            stamp: Rank,
            churn: DirChurn,
        }
        let ins = self.insights();
        let src = ins.source();
        let mut dirs: Vec<Acc> = Vec::new();
        let mut dir_slot: FastMap<String, usize> = FastMap::default();
        // Directory slot per path id, resolved once. A path belongs to one directory, so the
        // first sight of a path is a new file of its directory.
        let mut path_dir: FastMap<PathId, usize> = FastMap::default();
        for (rank, _, rows) in self.commit_rows() {
            for f in &rows {
                let Some(l) = ins.row_lines(f) else {
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

    /// The counted commits with their kept rows and author time, streamed.
    fn commit_rows(&self) -> impl Iterator<Item = (Rank, i64, Vec<FileRow>)> + '_ {
        let src = self.insights().source();
        self.rows()
            .filter_map(move |(r, rows)| src.commit(r).map(|c| (r, c.time, rows)))
    }
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

    /// Types, scopes and churn count only the week's own local days.
    #[test]
    fn labels_and_churn_count_only_the_local_days_of_the_period() {
        let recs = crate::insights::code::tests::edge_week();
        let ins = Insights::new(&recs, Options::default(), None);
        let p = Period::Week;
        let n: usize = ins
            .label_breakdown(Label::Kind, p)
            .iter()
            .map(|l| l.commits)
            .sum();
        assert_eq!(
            n, 2,
            "commit on local day 13 counted in a week of days 14..=20"
        );
        let over: usize = ins
            .label_over_time(Label::Kind, p)
            .iter()
            .flat_map(|b| &b.counts)
            .map(|l| l.commits)
            .sum();
        assert_eq!(over, 2);
        assert_eq!(ins.counted_rows(p).count(), 2);
        let f = ins.file_churn(p);
        assert_eq!((f[0].commits, f[0].additions), (2, 15));
        assert_eq!(ins.dir_churn(p)[0].commits, 2);
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
