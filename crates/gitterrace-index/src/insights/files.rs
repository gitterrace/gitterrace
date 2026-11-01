use std::collections::{BTreeMap, HashMap, HashSet};

use super::{HeadFile, Insights, Period, year_of};

/// Most files in one commit that still say something about coupling; bigger commits are
/// sweeps (formatting, renames, vendoring).
const COUPLING_MAX_FILES: usize = 30;
const SOLO_SHARE: f64 = 0.8;
const STALE_SECS: i64 = 365 * 86_400;

/// One change to a file, kept per canonical (head) path.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TouchRaw {
    pub commit: u32,
    /// Index into the commit's `files`, which holds the name at that time.
    pub file: u32,
    pub lines: u32,
}

/// Every non-merge commit that touched a path, followed back through renames to the name the
/// file has at the newest commit.
#[derive(Default)]
pub(crate) struct Follow {
    pub touches: HashMap<String, Vec<TouchRaw>>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Touch {
    pub commit: String,
    pub time: i64,
    pub author: String,
    pub lines: u32,
    /// Name the file had in that commit.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Hotspot {
    pub path: String,
    pub language: String,
    /// Lines now.
    pub lines: u32,
    /// Commits that touched the file in the period.
    pub changes: usize,
    /// Added plus removed lines in the period.
    pub lines_changed: u64,
    pub authors: usize,
    pub last_change: i64,
    /// `changes * sqrt(lines)`.
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HotspotReport {
    /// Sorted by score. The same rows feed the scatter plot (x changes, y lines, size authors).
    pub files: Vec<Hotspot>,
    pub total_changes: usize,
    /// Share of all changes that hit the top 10% of files.
    pub concentration: f64,
    pub top_tenth: usize,
    pub single_author_files: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Owner {
    pub name: String,
    /// Share of the knowledge, 0..=1.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DirOwnership {
    /// `""` for the whole repository.
    pub path: String,
    pub lines: u64,
    pub files: usize,
    pub owners: Vec<Owner>,
    /// People who together hold at least half of the knowledge.
    pub bus_factor: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OwnershipReport {
    pub overall: DirOwnership,
    pub dirs: Vec<DirOwnership>,
    /// Lines in files where one person made at least 80% of the changes.
    pub solo_lines: u64,
    /// Lines whose main owner has no commit in the 12 months before the newest commit.
    pub stale_lines: u64,
    /// Lines in files with no change in the period, not counted above.
    pub untracked_lines: u64,
    pub dirs_with_bus_factor_one: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Coupling {
    pub a: String,
    pub b: String,
    /// Commits that changed both.
    pub together: usize,
    pub a_changes: usize,
    pub b_changes: usize,
    /// `together / min(a_changes, b_changes)`.
    pub degree: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgeBucket {
    pub year: i64,
    pub lines: u64,
    pub files: usize,
}

/// Lines now by the year their file was last touched. This is a blame-free approximation:
/// a file counts fully toward its newest change, so old code in a recently edited file looks
/// younger than it is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgeReport {
    pub buckets: Vec<AgeBucket>,
    pub total_lines: u64,
}

impl Insights<'_> {
    pub(crate) fn build_follow(&self) -> Follow {
        let mut alias: HashMap<&str, &str> = HashMap::new();
        let mut touches: HashMap<String, Vec<TouchRaw>> = HashMap::new();
        for i in (0..self.records.len()).rev() {
            let r = &self.records[i];
            if r.merge {
                continue;
            }
            for (fi, f) in r.files.iter().enumerate() {
                let path = alias
                    .get(f.path.as_str())
                    .copied()
                    .unwrap_or(f.path.as_str());
                if let Some(old) = &f.old {
                    alias.insert(old.as_str(), path);
                }
                let l = self.lines_unfiltered(f);
                touches.entry(path.to_owned()).or_default().push(TouchRaw {
                    commit: u32::try_from(i).unwrap_or(u32::MAX),
                    file: u32::try_from(fi).unwrap_or(u32::MAX),
                    lines: l.add + l.del,
                });
            }
        }
        Follow { touches }
    }

    /// Touches of `path` in the period by counted commits, newest first.
    fn touches_in(
        &self,
        path: &str,
        from: i64,
        to: i64,
    ) -> impl Iterator<Item = &super::files::TouchRaw> {
        self.follow()
            .touches
            .get(path)
            .into_iter()
            .flatten()
            .filter(move |t| {
                let i = t.commit as usize;
                let time = self.records[i].time;
                time >= from && time < to && self.counted(i)
            })
    }

    /// Full history of a file as it is named at the newest commit, following renames.
    /// Includes bots and every non-merge commit, newest first.
    pub fn file_history(&self, path: &str) -> Vec<Touch> {
        let mut out: Vec<Touch> = self
            .follow()
            .touches
            .get(path)
            .into_iter()
            .flatten()
            .map(|t| {
                let r = &self.records[t.commit as usize];
                Touch {
                    commit: r.id.clone(),
                    time: r.time,
                    author: self.people.list[self.person[t.commit as usize]]
                        .name
                        .clone(),
                    lines: t.lines,
                    path: r.files[t.file as usize].path.clone(),
                }
            })
            .collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.time));
        out
    }

    /// Hotspots among `files` (the head tree) for the period.
    pub fn hotspots(&self, files: &[HeadFile], period: Period) -> HotspotReport {
        let (from, to) = period.window(self.now);
        let mut rows = Vec::new();
        for f in files {
            if f.lines == 0 || self.filter.skips(&f.path) {
                continue;
            }
            let mut count = 0;
            let mut churn = 0u64;
            let mut last = i64::MIN;
            let mut who = HashSet::new();
            for t in self.touches_in(&f.path, from, to) {
                count += 1;
                churn += u64::from(t.lines);
                last = last.max(self.records[t.commit as usize].time);
                who.insert(self.person[t.commit as usize]);
            }
            if count == 0 {
                continue;
            }
            rows.push(Hotspot {
                path: f.path.clone(),
                language: super::language(&f.path),
                lines: f.lines,
                changes: count,
                lines_changed: churn,
                authors: who.len(),
                last_change: last,
                score: count as f64 * f64::from(f.lines).sqrt(),
            });
        }
        rows.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.path.cmp(&b.path))
        });
        let total: usize = rows.iter().map(|r| r.changes).sum();
        let tenth = rows.len().div_ceil(10).max(1);
        let mut by_changes: Vec<usize> = rows.iter().map(|r| r.changes).collect();
        by_changes.sort_unstable_by(|a, b| b.cmp(a));
        let top: usize = by_changes.iter().take(tenth).sum();
        HotspotReport {
            concentration: if total == 0 {
                0.0
            } else {
                top as f64 / total as f64
            },
            top_tenth: tenth,
            single_author_files: rows.iter().filter(|r| r.authors == 1).count(),
            total_changes: total,
            files: rows,
        }
    }

    /// Knowledge share per file, weighted by its size now, summed per directory `depth` levels
    /// deep (`depth` 1 groups by top-level directory; root files group under `""`).
    pub fn ownership(&self, files: &[HeadFile], period: Period, depth: usize) -> OwnershipReport {
        let (from, to) = period.window(self.now);
        let mut dirs: BTreeMap<String, Acc> = BTreeMap::new();
        let mut all = Acc::default();
        let (mut solo, mut stale, mut untracked) = (0u64, 0u64, 0u64);
        for f in files {
            if f.lines == 0 || self.filter.skips(&f.path) {
                continue;
            }
            let mut who: HashMap<usize, u64> = HashMap::new();
            let mut changed = 0u64;
            for t in self.touches_in(&f.path, from, to) {
                *who.entry(self.person[t.commit as usize]).or_default() += u64::from(t.lines);
                changed += u64::from(t.lines);
            }
            if changed == 0 {
                untracked += u64::from(f.lines);
                continue;
            }
            let parts: Vec<&str> = f.path.split('/').collect();
            let key = if parts.len() > 1 {
                parts[..depth.min(parts.len() - 1)].join("/")
            } else {
                String::new()
            };
            let d = dirs.entry(key).or_default();
            let (mut top_p, mut top_w) = (0, 0.0);
            for (p, c) in who {
                let w = f64::from(f.lines) * c as f64 / changed as f64;
                *d.w.entry(p).or_default() += w;
                *all.w.entry(p).or_default() += w;
                if w > top_w {
                    top_w = w;
                    top_p = p;
                }
            }
            d.lines += u64::from(f.lines);
            d.files += 1;
            all.lines += u64::from(f.lines);
            all.files += 1;
            if top_w / f64::from(f.lines) >= SOLO_SHARE {
                solo += u64::from(f.lines);
            }
            if self.last_seen[top_p] < self.now - STALE_SECS {
                stale += u64::from(f.lines);
            }
        }
        let finish = |path: String, a: Acc| -> DirOwnership {
            let mut owners: Vec<(usize, f64)> = a.w.into_iter().collect();
            owners.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
            let total: f64 = owners
                .iter()
                .map(|o| o.1)
                .sum::<f64>()
                .max(f64::MIN_POSITIVE);
            let bus = crate::aggregates::bus_factor(owners.iter().map(|o| o.1));
            DirOwnership {
                path,
                lines: a.lines,
                files: a.files,
                bus_factor: bus,
                owners: owners
                    .into_iter()
                    .map(|(p, w)| Owner {
                        name: self.people.list[p].name.clone(),
                        share: w / total,
                    })
                    .collect(),
            }
        };
        let mut dirs: Vec<DirOwnership> = dirs.into_iter().map(|(k, a)| finish(k, a)).collect();
        dirs.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.path.cmp(&b.path)));
        OwnershipReport {
            dirs_with_bus_factor_one: dirs
                .iter()
                .filter(|d| d.bus_factor == 1 && !d.owners.is_empty())
                .count(),
            overall: finish(String::new(), all),
            dirs,
            solo_lines: solo,
            stale_lines: stale,
            untracked_lines: untracked,
        }
    }

    /// Pairs of head files that changed in the same commits, strongest first. Pairs need at
    /// least `min_together` shared commits. Commits touching more than 30 files are skipped.
    pub fn coupling(
        &self,
        files: &[HeadFile],
        period: Period,
        min_together: usize,
        limit: usize,
    ) -> Vec<Coupling> {
        let (from, to) = period.window(self.now);
        let head: HashSet<&str> = files
            .iter()
            .filter(|f| !self.filter.skips(&f.path))
            .map(|f| f.path.as_str())
            .collect();
        // Commit -> its head files, from the follow index so renames are joined.
        let mut per_commit: HashMap<u32, Vec<&str>> = HashMap::new();
        let mut changes: HashMap<&str, usize> = HashMap::new();
        for path in &head {
            for t in self.touches_in(path, from, to) {
                per_commit.entry(t.commit).or_default().push(path);
                *changes.entry(path).or_default() += 1;
            }
        }
        let mut pairs: HashMap<(&str, &str), usize> = HashMap::new();
        for mut group in per_commit.into_values() {
            if group.len() < 2 || group.len() > COUPLING_MAX_FILES {
                continue;
            }
            group.sort_unstable();
            for (x, a) in group.iter().enumerate() {
                for b in &group[x + 1..] {
                    *pairs.entry((a, b)).or_default() += 1;
                }
            }
        }
        let mut out: Vec<Coupling> = pairs
            .into_iter()
            .filter(|(_, n)| *n >= min_together)
            .map(|((a, b), together)| {
                let (ac, bc) = (changes[a], changes[b]);
                Coupling {
                    a: a.to_owned(),
                    b: b.to_owned(),
                    together,
                    a_changes: ac,
                    b_changes: bc,
                    degree: together as f64 / ac.min(bc) as f64,
                }
            })
            .collect();
        out.sort_by(|x, y| {
            y.together
                .cmp(&x.together)
                .then_with(|| y.degree.total_cmp(&x.degree))
                .then_with(|| x.a.cmp(&y.a))
                .then_with(|| x.b.cmp(&y.b))
        });
        out.truncate(limit);
        out
    }

    /// Lines now grouped by the year of the file's last change. See [`AgeReport`].
    pub fn code_age(&self, files: &[HeadFile]) -> AgeReport {
        let mut years: BTreeMap<i64, (u64, usize)> = BTreeMap::new();
        let mut total = 0;
        for f in files {
            if f.lines == 0 || self.filter.skips(&f.path) {
                continue;
            }
            let newest = self
                .touches_in(&f.path, i64::MIN, i64::MAX)
                .map(|t| t.commit)
                .max();
            let Some(c) = newest else { continue };
            let e = years.entry(year_of(&self.records[c as usize])).or_default();
            e.0 += u64::from(f.lines);
            e.1 += 1;
            total += u64::from(f.lines);
        }
        AgeReport {
            buckets: years
                .into_iter()
                .map(|(year, (lines, files))| AgeBucket { year, lines, files })
                .collect(),
            total_lines: total,
        }
    }
}

#[derive(Default)]
struct Acc {
    lines: u64,
    files: usize,
    w: HashMap<usize, f64>,
}

#[cfg(test)]
mod tests {
    use super::super::HeadFile;
    use crate::insights::fixtures::{file, rec};
    use crate::{FileStat, Insights, Options, Period};

    fn head(files: &[(&str, u32)]) -> Vec<HeadFile> {
        files
            .iter()
            .map(|(p, n)| HeadFile {
                path: (*p).into(),
                lines: *n,
            })
            .collect()
    }

    fn renamed(old: &str, new: &str) -> FileStat {
        FileStat {
            old: Some(old.into()),
            ..file(new, 0, 0, 10)
        }
    }

    fn history() -> Vec<crate::CommitRecord> {
        vec![
            rec(
                "a",
                "Ann",
                0,
                vec![file("old/x.rs", 10, 0, 10), file("keep.rs", 4, 0, 4)],
            ),
            rec("b", "Bob", 400, vec![file("old/x.rs", 2, 1, 11)]),
            rec(
                "c",
                "Ann",
                401,
                vec![renamed("old/x.rs", "new/x.rs"), file("keep.rs", 1, 1, 4)],
            ),
            rec(
                "d",
                "Ann",
                402,
                vec![file("new/x.rs", 3, 0, 14), file("keep.rs", 1, 0, 5)],
            ),
        ]
    }

    #[test]
    fn hotspots_follow_renames() {
        let recs = history();
        let ins = Insights::new(&recs, Options::default(), None);
        let rep = ins.hotspots(&head(&[("new/x.rs", 14), ("keep.rs", 5)]), Period::All);
        let x = rep
            .files
            .iter()
            .find(|f| f.path == "new/x.rs")
            .map(|f| (f.changes, f.authors));
        assert_eq!(x, Some((4, 2)));
        assert_eq!(rep.files[0].path, "new/x.rs");
        assert!((rep.files[0].score - 4.0 * 14f64.sqrt()).abs() < 1e-9);
        assert_eq!(rep.total_changes, 7);
        assert_eq!(rep.single_author_files, 1);
        let h = ins.file_history("new/x.rs");
        assert_eq!(h.len(), 4);
        assert_eq!(h[0].commit, "d");
    }

    #[test]
    fn ownership_and_bus_factor() {
        let recs = history();
        let ins = Insights::new(&recs, Options::default(), None);
        let o = ins.ownership(&head(&[("new/x.rs", 14), ("keep.rs", 5)]), Period::All, 1);
        // new/x.rs: Ann 10+0+3 = 13 changed lines, Bob 3. keep.rs: Ann only.
        let nx = o
            .dirs
            .iter()
            .find(|d| d.path == "new")
            .map(|d| d.owners[0].name.clone());
        assert_eq!(nx.as_deref(), Some("Ann"));
        assert_eq!(o.overall.bus_factor, 1);
        let root = o
            .dirs
            .iter()
            .find(|d| d.path.is_empty())
            .map(|d| d.bus_factor);
        assert_eq!(root, Some(1));
        let total: f64 = o.overall.owners.iter().map(|x| x.share).sum();
        assert!((total - 1.0).abs() < 1e-9);
        // Ann made 13 of the 16 changed lines of new/x.rs (81%), so both files are solo.
        assert_eq!(o.solo_lines, 19);
        assert_eq!(o.stale_lines, 0);
    }

    #[test]
    fn coupling_and_age() {
        let recs = history();
        let ins = Insights::new(&recs, Options::default(), None);
        let hf = head(&[("new/x.rs", 14), ("keep.rs", 5)]);
        let c = ins.coupling(&hf, Period::All, 2, 10);
        assert_eq!(c.len(), 1);
        assert_eq!(
            (c[0].together, c[0].a.as_str(), c[0].b.as_str()),
            (3, "keep.rs", "new/x.rs")
        );
        assert!((c[0].degree - 1.0).abs() < 1e-9);
        let age = ins.code_age(&hf);
        assert_eq!(age.total_lines, 19);
        assert_eq!(age.buckets.len(), 1);
        assert_eq!(age.buckets[0].year, 2025);
    }

    #[test]
    fn bots_are_excluded_by_default() {
        let recs = vec![
            rec("a", "Ann", 0, vec![file("a.rs", 5, 0, 5)]),
            rec("b", "dependabot[bot]", 1, vec![file("a.rs", 5, 0, 10)]),
        ];
        let hf = head(&[("a.rs", 10)]);
        let ins = Insights::new(&recs, Options::default(), None);
        assert_eq!(ins.hotspots(&hf, Period::All).files[0].changes, 1);
        let with = Insights::new(
            &recs,
            Options {
                include_bots: true,
                ..Options::default()
            },
            None,
        );
        assert_eq!(with.hotspots(&hf, Period::All).files[0].changes, 2);
    }
}
