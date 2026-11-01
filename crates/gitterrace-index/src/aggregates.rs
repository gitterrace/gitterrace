//! Pure statistics over cached commit records. Times are bucketed in each commit's own UTC
//! offset, so a commit made at 23:30 local counts as late evening whoever reads the page.

use std::collections::{BTreeSet, HashMap};

use gitterrace_git::civil_from_days as civil;

use crate::insights::fasthash::{FastMap, FastSet};
use crate::{CommitRecord, TimeAxis};

pub(crate) const DAY: i64 = 86_400;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Totals {
    pub commits: usize,
    pub authors: usize,
    pub files_touched: usize,
    pub additions: u64,
    pub deletions: u64,
    pub first_commit: Option<i64>,
    pub last_commit: Option<i64>,
}

/// One bucket of a time series. `key` is the `YYYY-MM-DD` the bucket starts on, `label` what an
/// axis shows for it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ActivityBucket {
    pub key: String,
    pub label: String,
    pub commits: usize,
    pub additions: u64,
    pub deletions: u64,
}

/// Commit counts by local weekday (Monday is 0) and local hour.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Heatmap {
    pub cells: [[u32; 24]; 7],
    pub max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AuthorStats {
    pub name: String,
    pub email: String,
    pub commits: usize,
    pub additions: u64,
    pub deletions: u64,
    pub first_commit: i64,
    pub last_commit: i64,
    pub active_days: usize,
}

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

pub fn totals(records: &[CommitRecord]) -> Totals {
    let mut t = Totals::default();
    let mut authors: FastSet<&str> = FastSet::default();
    let mut files: FastSet<&str> = FastSet::default();
    for r in records {
        authors.insert(r.email.as_str());
        for f in &r.files {
            files.insert(&*f.path);
            t.additions += u64::from(f.additions);
            t.deletions += u64::from(f.deletions);
        }
        t.first_commit = Some(t.first_commit.map_or(r.time, |v| v.min(r.time)));
        t.last_commit = Some(t.last_commit.map_or(r.time, |v| v.max(r.time)));
    }
    t.commits = records.len();
    t.authors = authors.len();
    t.files_touched = files.len();
    t
}

/// One bucket per axis slot, empty ones included.
pub fn activity(records: &[CommitRecord], axis: &TimeAxis) -> Vec<ActivityBucket> {
    let mut out: Vec<ActivityBucket> = axis
        .names()
        .map(|(key, label)| ActivityBucket {
            key,
            label,
            commits: 0,
            additions: 0,
            deletions: 0,
        })
        .collect();
    for r in records {
        let Some(b) = axis.slot(r).map(|i| &mut out[i]) else {
            continue;
        };
        b.commits += 1;
        let (a, d) = line_totals(r);
        b.additions += a;
        b.deletions += d;
    }
    out
}

pub fn heatmap(records: &[CommitRecord]) -> Heatmap {
    let mut cells = [[0u32; 24]; 7];
    for r in records {
        let local = local_time(r);
        let days = local.div_euclid(DAY);
        let weekday = weekday(days);
        let hour = usize::try_from(local.rem_euclid(DAY) / 3600).unwrap_or(0);
        cells[weekday][hour] += 1;
    }
    let max = cells.iter().flatten().copied().max().unwrap_or(0);
    Heatmap { cells, max }
}

/// Bus factor: the fewest contributors whose shares, largest first, hold at
/// least half of the total. Shares in any order; zero when there are none.
/// The one definition every surface (insights, API, MCP, cards) uses.
pub fn bus_factor(shares: impl IntoIterator<Item = f64>) -> usize {
    let mut v: Vec<f64> = shares.into_iter().filter(|s| *s > 0.0).collect();
    v.sort_by(|a, b| b.total_cmp(a));
    let total: f64 = v.iter().sum();
    let mut acc = 0.0;
    for (i, s) in v.iter().enumerate() {
        acc += s;
        if acc * 2.0 >= total {
            return i + 1;
        }
    }
    0
}

/// Authors keyed by email, most commits first, then name.
pub fn authors(records: &[CommitRecord]) -> Vec<AuthorStats> {
    struct Acc {
        stats: AuthorStats,
        days: BTreeSet<i64>,
        name_time: i64,
    }
    let mut map: HashMap<&str, Acc> = HashMap::new();
    for r in records {
        let (a, d) = line_totals(r);
        let acc = map.entry(r.email.as_str()).or_insert_with(|| Acc {
            stats: AuthorStats {
                name: r.author.clone(),
                email: r.email.clone(),
                commits: 0,
                additions: 0,
                deletions: 0,
                first_commit: r.time,
                last_commit: r.time,
                active_days: 0,
            },
            days: BTreeSet::new(),
            name_time: r.time,
        });
        acc.stats.commits += 1;
        acc.stats.additions += a;
        acc.stats.deletions += d;
        acc.stats.first_commit = acc.stats.first_commit.min(r.time);
        acc.stats.last_commit = acc.stats.last_commit.max(r.time);
        // The most recent spelling of the name wins.
        if r.time > acc.name_time {
            acc.name_time = r.time;
            acc.stats.name.clone_from(&r.author);
        }
        acc.days.insert(local_time(r).div_euclid(DAY));
    }
    let mut out: Vec<AuthorStats> = map
        .into_values()
        .map(|mut acc| {
            acc.stats.active_days = acc.days.len();
            acc.stats
        })
        .collect();
    out.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Every file with its churn, most commits first, then path.
pub fn file_churn(records: &[CommitRecord]) -> Vec<FileChurn> {
    churn_since(records, i64::MIN, |_| true)
}

/// The `limit` most changed files among commits at or after `since` (epoch seconds), leaving
/// out generated files (lock files, minified output) as the web hotspots page does.
pub fn hotspots(records: &[CommitRecord], since: i64, limit: usize) -> Vec<FileChurn> {
    let mut out = churn_since(records, since, |p| !crate::is_generated(p));
    out.truncate(limit);
    out
}

/// Churn per containing directory, most commits first, then path.
pub fn dir_churn(records: &[CommitRecord]) -> Vec<DirChurn> {
    struct Acc {
        commits: usize,
        files: usize,
        additions: u64,
        deletions: u64,
    }
    let mut map: FastMap<&str, Acc> = FastMap::default();
    // A path belongs to one directory, so first sight of a path is a new file of its directory.
    let mut paths: FastSet<&str> = FastSet::default();
    for r in records {
        let mut touched: Vec<&str> = Vec::new();
        for f in &r.files {
            let dir = f.path.rsplit_once('/').map_or(".", |(d, _)| d);
            let acc = map.entry(dir).or_insert_with(|| Acc {
                commits: 0,
                files: 0,
                additions: 0,
                deletions: 0,
            });
            if !touched.contains(&dir) {
                touched.push(dir);
                acc.commits += 1;
            }
            if paths.insert(&*f.path) {
                acc.files += 1;
            }
            acc.additions += u64::from(f.additions);
            acc.deletions += u64::from(f.deletions);
        }
    }
    let mut out: Vec<DirChurn> = map
        .into_iter()
        .map(|(path, a)| DirChurn {
            path: path.to_owned(),
            commits: a.commits,
            files: a.files,
            additions: a.additions,
            deletions: a.deletions,
        })
        .collect();
    out.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.path.cmp(&b.path)));
    out
}

/// Commits per type or scope. Commits without the field are left out.
pub fn label_breakdown(records: &[CommitRecord], label: Label) -> Vec<LabelCount> {
    let mut map: HashMap<&str, usize> = HashMap::new();
    for r in records {
        if let Some(l) = label_of(r, label) {
            *map.entry(l).or_default() += 1;
        }
    }
    sorted_counts(map)
}

/// Type or scope counts per axis slot, empty slots included.
pub fn label_over_time(
    records: &[CommitRecord],
    label: Label,
    axis: &TimeAxis,
) -> Vec<LabelBucket> {
    let mut maps: Vec<HashMap<&str, usize>> = vec![HashMap::new(); axis.len()];
    for r in records {
        if let (Some(l), Some(i)) = (label_of(r, label), axis.slot(r)) {
            *maps[i].entry(l).or_default() += 1;
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

fn churn_since(
    records: &[CommitRecord],
    since: i64,
    keep: impl Fn(&str) -> bool,
) -> Vec<FileChurn> {
    let mut map: FastMap<&str, FileChurn> = FastMap::default();
    for r in records.iter().filter(|r| r.time >= since) {
        // A commit touches few files, so a scan beats hashing for the once-per-commit check.
        let mut seen: Vec<&str> = Vec::new();
        for f in r.files.iter().filter(|f| keep(&f.path)) {
            let c = map.entry(&*f.path).or_insert_with(|| FileChurn {
                path: f.path.to_string(),
                commits: 0,
                additions: 0,
                deletions: 0,
                last_commit: r.time,
            });
            if !seen.contains(&&*f.path) {
                seen.push(&*f.path);
                c.commits += 1;
            }
            c.additions += u64::from(f.additions);
            c.deletions += u64::from(f.deletions);
            c.last_commit = c.last_commit.max(r.time);
        }
    }
    let mut out: Vec<FileChurn> = map.into_values().collect();
    out.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.path.cmp(&b.path)));
    out
}

fn label_of(r: &CommitRecord, label: Label) -> Option<&str> {
    match label {
        Label::Kind => r.kind.as_deref(),
        Label::Scope => r.scope.as_deref(),
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

pub(crate) fn line_totals(r: &CommitRecord) -> (u64, u64) {
    r.files.iter().fold((0, 0), |(a, d), f| {
        (a + u64::from(f.additions), d + u64::from(f.deletions))
    })
}

pub(crate) fn local_time(r: &CommitRecord) -> i64 {
    r.time + i64::from(r.offset)
}

/// Monday is 0. Day 0 (1970-01-01) was a Thursday.
pub(crate) fn weekday(days: i64) -> usize {
    usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0)
}

pub(crate) fn format_date(days: i64) -> String {
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use crate::aggregates::{
        Label, activity, authors, bus_factor, dir_churn, file_churn, heatmap, hotspots,
        label_breakdown, label_over_time, totals,
    };
    use crate::{CommitRecord, FileStat, Period, TimeAxis};

    #[test]
    fn bus_factor_counts_who_hold_at_least_half() {
        // The 50/50 split is the case the surfaces used to disagree on.
        assert_eq!(bus_factor([5.0, 5.0]), 1);
        assert_eq!(bus_factor([6.0, 3.0, 1.0]), 1);
        assert_eq!(bus_factor([3.0, 3.0, 3.0, 1.0]), 2);
        assert_eq!(bus_factor([1.0, 4.0]), 1);
        assert_eq!(bus_factor([0.0, 0.0]), 0);
        assert_eq!(bus_factor(std::iter::empty()), 0);
    }

    // 2024-01-01 is a Monday.
    const MON: i64 = 1_704_067_200;
    const HOUR: i64 = 3600;

    fn rec(id: &str, who: &str, time: i64, files: &[(&str, u32, u32)]) -> CommitRecord {
        CommitRecord {
            id: id.into(),
            author: who.into(),
            email: format!("{who}@x"),
            time,
            offset: 0,
            merge: false,
            kind: None,
            scope: None,
            files: files
                .iter()
                .map(|(p, a, d)| FileStat {
                    path: (*p).into(),
                    additions: *a,
                    deletions: *d,
                    ..FileStat::default()
                })
                .collect(),
            ..CommitRecord::default()
        }
    }

    fn typed(mut r: CommitRecord, kind: &str, scope: Option<&str>) -> CommitRecord {
        r.kind = Some(kind.into());
        r.scope = scope.map(Into::into);
        r
    }

    fn sample() -> Vec<CommitRecord> {
        vec![
            rec(
                "1",
                "ann",
                MON + 9 * HOUR,
                &[("src/a.rs", 10, 0), ("README.md", 1, 0)],
            ),
            rec("2", "bob", MON + DAYS(1) + 10 * HOUR, &[("src/a.rs", 3, 2)]),
            rec(
                "3",
                "ann",
                MON + DAYS(8) + 23 * HOUR,
                &[("src/b.rs", 5, 5), ("src/a.rs", 1, 1)],
            ),
        ]
    }

    #[allow(non_snake_case)]
    fn DAYS(n: i64) -> i64 {
        n * 86_400
    }

    #[test]
    fn totals_of_empty_and_sample() {
        assert_eq!(totals(&[]).commits, 0);
        assert_eq!(totals(&[]).first_commit, None);
        let t = totals(&sample());
        assert_eq!((t.commits, t.authors, t.files_touched), (3, 2, 3));
        assert_eq!((t.additions, t.deletions), (20, 8));
        assert_eq!(t.first_commit, Some(MON + 9 * HOUR));
        assert_eq!(t.last_commit, Some(MON + DAYS(8) + 23 * HOUR));
    }

    fn axis(r: &[CommitRecord], p: Option<Period>) -> TimeAxis {
        TimeAxis::covering(p, r, r)
    }

    #[test]
    fn activity_fills_the_axis_with_empty_buckets() {
        let r = sample();
        // Commits on 2024-01-01, 01-02 and 01-09: ten days, one gap of six empty days.
        let days = activity(&r, &axis(&r, None));
        let keys: Vec<_> = days.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(days.len(), 9);
        assert_eq!((keys[0], keys[8]), ("2024-01-01", "2024-01-09"));
        assert_eq!(days[0].label, "Mon 1");
        let commits: Vec<_> = days.iter().map(|b| b.commits).collect();
        assert_eq!(commits, [1, 1, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(days[0].additions, 11);
    }

    #[test]
    fn a_week_selection_has_seven_day_columns() {
        let r = sample();
        let week = activity(&r, &axis(&r, Some(Period::Week)));
        assert_eq!(week.len(), 7);
        assert_eq!(week[6].commits, 1);
    }

    #[test]
    fn offset_moves_commit_across_the_day() {
        let mut r = rec("o", "a", MON - HOUR, &[]);
        assert_eq!(
            activity(
                std::slice::from_ref(&r),
                &axis(std::slice::from_ref(&r), None)
            )[0]
            .key,
            "2023-12-31"
        );
        r.offset = 2 * 3600;
        assert_eq!(
            activity(
                std::slice::from_ref(&r),
                &axis(std::slice::from_ref(&r), None)
            )[0]
            .key,
            "2024-01-01"
        );
    }

    #[test]
    fn heatmap_uses_local_time() {
        let mut late = rec("l", "a", MON + 23 * HOUR, &[]);
        late.offset = 2 * 3600;
        let h = heatmap(&[
            late,
            rec("m", "a", MON + 9 * HOUR, &[]),
            rec("n", "a", MON + 9 * HOUR, &[]),
        ]);
        assert_eq!(h.cells[0][9], 2);
        assert_eq!(h.cells[1][1], 1);
        assert_eq!(h.max, 2);
        assert_eq!(heatmap(&[]).max, 0);
    }

    #[test]
    fn heatmap_handles_pre_epoch() {
        // 1969-12-31 is a Wednesday.
        let h = heatmap(&[rec("p", "a", -HOUR, &[])]);
        assert_eq!(h.cells[2][23], 1);
    }

    #[test]
    fn authors_table() {
        let a = authors(&sample());
        assert_eq!(a[0].name, "ann");
        assert_eq!((a[0].commits, a[0].additions, a[0].deletions), (2, 17, 6));
        assert_eq!(a[0].active_days, 2);
        assert_eq!(a[0].first_commit, MON + 9 * HOUR);
        assert_eq!(a[0].last_commit, MON + DAYS(8) + 23 * HOUR);
        assert_eq!((a[1].name.as_str(), a[1].commits), ("bob", 1));
    }

    #[test]
    fn author_active_days_count_distinct_days() {
        let r = [rec("1", "a", MON, &[]), rec("2", "a", MON + HOUR, &[])];
        assert_eq!(authors(&r)[0].active_days, 1);
    }

    #[test]
    fn file_and_dir_churn() {
        let f = file_churn(&sample());
        assert_eq!(f[0].path, "src/a.rs");
        assert_eq!((f[0].commits, f[0].additions, f[0].deletions), (3, 14, 3));
        assert_eq!(f[0].last_commit, MON + DAYS(8) + 23 * HOUR);
        let d = dir_churn(&sample());
        assert_eq!(d[0].path, "src");
        assert_eq!((d[0].commits, d[0].files), (3, 2));
        assert_eq!(d[1].path, ".");
    }

    #[test]
    fn same_file_twice_in_one_commit_counts_one_commit() {
        let r = rec("1", "a", MON, &[("x", 1, 0), ("x", 2, 0)]);
        assert_eq!(file_churn(&[r])[0].commits, 1);
    }

    #[test]
    fn hotspots_leave_out_generated_files() {
        let mut r = sample();
        for rec in &mut r {
            rec.files.push(crate::FileStat {
                path: "Cargo.lock".into(),
                additions: 1,
                ..crate::FileStat::default()
            });
        }
        assert!(
            hotspots(&r, i64::MIN, 100)
                .iter()
                .all(|f| f.path != "Cargo.lock")
        );
        assert!(file_churn(&r).iter().any(|f| f.path == "Cargo.lock"));
    }

    #[test]
    fn hotspots_respect_window_and_limit() {
        let h = hotspots(&sample(), MON + DAYS(2), 1);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].path, "src/a.rs");
        assert_eq!(h[0].commits, 1);
        assert!(hotspots(&sample(), i64::MAX, 5).is_empty());
    }

    #[test]
    fn type_and_scope_breakdown() {
        let r = vec![
            typed(rec("1", "a", MON, &[]), "feat", Some("web")),
            typed(rec("2", "a", MON + DAYS(40), &[]), "feat", None),
            typed(rec("3", "a", MON + DAYS(41), &[]), "fix", Some("web")),
            rec("4", "a", MON, &[]),
        ];
        let kinds = label_breakdown(&r, Label::Kind);
        assert_eq!((kinds[0].label.as_str(), kinds[0].commits), ("feat", 2));
        assert_eq!(kinds.len(), 2);
        let scopes = label_breakdown(&r, Label::Scope);
        assert_eq!((scopes[0].label.as_str(), scopes[0].commits), ("web", 2));
        let over = label_over_time(&r, Label::Kind, &axis(&r, None));
        assert_eq!(over.len(), 42);
        assert_eq!(over[0].key, "2024-01-01");
        assert_eq!(over[0].counts.len(), 1);
        assert_eq!(over[1].counts.len(), 0);
        assert_eq!(over[0].label, "Mon 1");
    }
}
