use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use gitterrace_index::feed::{CommitRange, CommitRow, InsightsSource, PathId, local_day_of};
use gitterrace_index::{civil_from_days as civil, days_from_civil, weekday};

const DAY: i64 = 86_400;

/// Everything the Repo Wrapped card shows, for one calendar year (commit-local time).
#[derive(Debug, Clone, PartialEq)]
pub struct WrappedStats {
    pub repo: String,
    pub year: i32,
    pub commits: usize,
    pub active_days: usize,
    pub longest_streak: usize,
    /// 0 = January.
    pub busiest_month: Option<usize>,
    /// 0 = Monday.
    pub busiest_weekday: Option<usize>,
    pub busiest_hour: Option<usize>,
    /// Name and commits, at most three.
    pub top_authors: Vec<(String, usize)>,
    /// Language name and its share of touched files, 0..=1.
    pub top_language: Option<(String, f64)>,
    pub top_dir: Option<(String, usize)>,
    pub biggest_commit: Option<BiggestCommit>,
    /// Commits per week from the first week of the year through the last.
    pub weekly: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BiggestCommit {
    pub lines: u64,
}

/// The card's numbers for `year` over `src`: commit columns, then the file rows of that year's
/// commits streamed, so memory is the year's tallies, never the rows.
pub fn wrapped_stats(repo: &str, src: &dyn InsightsSource, year: i32) -> WrappedStats {
    let rs: Vec<CommitRow> = src
        .commits_in(CommitRange::All)
        .filter(|c| civil(local_day_of(c.time, c.offset)).0 == i64::from(year))
        .collect();
    let mut days = BTreeSet::new();
    let mut months = [0u32; 12];
    let mut weekdays = [0u32; 7];
    let mut hours = [0u32; 24];
    // Raw identity strings by email: the first name seen for an address names it.
    let mut authors: HashMap<Arc<str>, (Arc<str>, usize)> = HashMap::new();
    let jan1 = days_from_civil(i64::from(year), 1, 1);
    let start = jan1 - weekday(jan1) as i64;
    let end = days_from_civil(i64::from(year), 12, 31);
    let weeks = usize::try_from((end - start) / 7 + 1).unwrap_or(53);
    let mut weekly = vec![0u32; weeks];
    for c in &rs {
        let l = c.time.saturating_add(i64::from(c.offset));
        let d = local_day_of(c.time, c.offset);
        days.insert(d);
        months[usize::try_from(civil(d).1 - 1).unwrap_or(0)] += 1;
        weekdays[weekday(d)] += 1;
        hours[usize::try_from(l.rem_euclid(DAY) / 3600).unwrap_or(0)] += 1;
        if let Some(w) = usize::try_from((d - start) / 7).ok().filter(|w| *w < weeks) {
            weekly[w] += 1;
        }
        let (name, email) = src.ident(c.ident).unwrap_or_default();
        authors.entry(email).or_insert((name, 0)).1 += 1;
    }
    let mut dirs: HashMap<String, usize> = HashMap::new();
    let mut dir_of: HashMap<PathId, Arc<str>> = HashMap::new();
    let mut langs: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut biggest: Option<BiggestCommit> = None;
    for (_, files) in src.rows_for(rs.iter().map(|c| c.rank).collect()) {
        let mut seen: HashSet<Arc<str>> = HashSet::new();
        let mut lines = 0u64;
        for f in &files {
            lines += u64::from(f.add) + u64::from(f.del);
            let dir = dir_of
                .entry(f.path)
                .or_insert_with(|| {
                    let path = src.path(f.path).unwrap_or_default();
                    path.rsplit_once('/').map_or(".", |(d, _)| d).into()
                })
                .clone();
            if seen.insert(Arc::clone(&dir)) {
                *dirs.entry(dir.to_string()).or_default() += 1;
            }
        }
        for f in &files {
            if let Some(l) = src.path(f.path).and_then(|p| language_of(&p)) {
                *langs.entry(l).or_default() += 1;
            }
        }
        if biggest.as_ref().is_none_or(|b| lines > b.lines) && !files.is_empty() {
            biggest = Some(BiggestCommit { lines });
        }
    }
    let mut top_authors: Vec<(String, usize)> = authors
        .into_values()
        .map(|(n, c)| (n.to_string(), c))
        .collect();
    top_authors.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top_authors.truncate(3);
    let top_dir = dirs
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
    let langs = languages(langs);
    WrappedStats {
        repo: repo.to_owned(),
        year,
        commits: rs.len(),
        active_days: days.len(),
        longest_streak: longest_streak(&days),
        busiest_month: argmax(&months),
        busiest_weekday: argmax(&weekdays),
        busiest_hour: argmax(&hours),
        top_authors,
        top_language: langs.into_iter().find(|(n, _)| n != "Other"),
        top_dir,
        biggest_commit: biggest,
        weekly,
    }
}

/// Share of file touches per language, largest first, small ones folded into "Other".
fn languages(map: BTreeMap<&'static str, usize>) -> Vec<(String, f64)> {
    let total: usize = map.values().sum();
    if total == 0 {
        return Vec::new();
    }
    let mut v: Vec<(&str, usize)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut out = Vec::new();
    let mut other = 0usize;
    for (i, (n, c)) in v.into_iter().enumerate() {
        if i < 6 && c * 50 >= total {
            out.push((n.to_owned(), c as f64 / total as f64));
        } else {
            other += c;
        }
    }
    if other > 0 {
        out.push(("Other".to_owned(), other as f64 / total as f64));
    }
    out
}

/// Programming languages only: docs, configs and lockfiles are not "languages".
fn language_of(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next()?;
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "Rust",
        "py" => "Python",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" => "C++",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "sh" | "bash" | "zsh" => "Shell",
        "html" | "htm" | "j2" => "HTML",
        "css" | "scss" => "CSS",
        "lua" => "Lua",
        "zig" => "Zig",
        "ex" | "exs" => "Elixir",
        "hs" => "Haskell",
        "sql" => "SQL",
        "svelte" => "Svelte",
        "vue" => "Vue",
        _ => return None,
    })
}

fn longest_streak(days: &BTreeSet<i64>) -> usize {
    let (mut best, mut cur, mut prev) = (0, 0, i64::MIN);
    for d in days {
        cur = if *d == prev + 1 { cur + 1 } else { 1 };
        best = best.max(cur);
        prev = *d;
    }
    best
}

/// Index of the maximum, first on ties; `None` when all zero.
fn argmax(v: &[u32]) -> Option<usize> {
    let (i, m) = v
        .iter()
        .enumerate()
        .fold((0, 0), |a, (i, &x)| if x > a.1 { (i, x) } else { a });
    (m > 0).then_some(i)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use gitterrace_index::feed::MemorySource;
    use gitterrace_index::{CommitRecord, FileStat};

    /// A record at `day` days after 2025-01-01 00:00 UTC plus `hour`.
    pub fn rec(i: usize, author: &str, day: i64, hour: i64, files: &[(&str, u32)]) -> CommitRecord {
        CommitRecord {
            id: format!("{i:040x}"),
            author: author.to_owned(),
            email: format!("{author}@x.io"),
            time: 1_735_689_600 + day * 86_400 + hour * 3600,
            offset: 0,
            merge: false,
            kind: None,
            scope: None,
            files: files
                .iter()
                .map(|(p, a)| FileStat {
                    path: (*p).into(),
                    additions: *a,
                    deletions: 1,
                    ..FileStat::default()
                })
                .collect(),
            ..CommitRecord::default()
        }
    }

    /// The sample history, through the oracle adapter (the card reads any source).
    pub fn sample() -> MemorySource<'static> {
        MemorySource::shared(std::sync::Arc::new(records()))
    }

    fn records() -> Vec<CommitRecord> {
        let mut v = Vec::new();
        for i in 0..40 {
            let who = if i % 4 == 0 { "bob" } else { "alice" };
            v.push(rec(
                i,
                who,
                (i as i64) * 3 / 2,
                9 + (i as i64 % 3),
                &[("src/main.rs", 10 + i as u32), ("README.md", 1)],
            ));
        }
        v.push(rec(
            99,
            "carol",
            20,
            14,
            &[("web/app.ts", 500), ("web/b.ts", 20)],
        ));
        v
    }
}

#[cfg(test)]
mod tests {
    use crate::stats::fixtures::sample;
    use crate::stats::wrapped_stats;

    #[test]
    fn wrapped_counts() {
        let w = wrapped_stats("demo", &sample(), 2025);
        assert_eq!(w.commits, 41);
        assert_eq!(w.top_authors[0], ("alice".to_owned(), 30));
        assert!(w.biggest_commit.as_ref().is_some_and(|b| b.lines > 500));
        assert_eq!(w.weekly.iter().sum::<u32>(), 41);
        assert_eq!(w.busiest_month, Some(0));
        assert!(w.longest_streak >= 2);
        assert_eq!(w.top_language.as_ref().map(|l| l.0.as_str()), Some("Rust"));
        assert_eq!(wrapped_stats("demo", &sample(), 2020).commits, 0);
    }
}
