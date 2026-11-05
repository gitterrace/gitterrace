//! The `a..b` comparison: commits, people, files and per-language change between two revisions.

use std::collections::{HashMap, HashSet};

use crate::feed::{PathId, Rank};
use crate::insights::{Insights, language};

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

/// Lines of one commit as git reports them, for a range the index does not hold.
pub struct RawCommit {
    pub id: String,
    /// The author, by email as the mailmap names them.
    pub author: String,
    pub merge: bool,
    /// `(path, added, removed, rewritten)` per file.
    pub files: Vec<(String, u64, u64, u64)>,
}

impl Comparison {
    /// `a..b` from commits read straight from git, newest first: a branch the index never
    /// walked (anything not reachable from the indexed head) still compares.
    pub fn of_raw(commits: &[RawCommit]) -> Self {
        let mut langs: HashMap<String, (u64, u64, u64)> = HashMap::new();
        let mut people = HashSet::new();
        let mut paths = HashSet::new();
        let (mut added, mut removed) = (0, 0);
        for c in commits.iter().filter(|c| !c.merge) {
            people.insert(c.author.as_str());
            for (path, a, d, r) in &c.files {
                paths.insert(path.as_str());
                let e = langs.entry(language(path)).or_default();
                e.0 += a;
                e.1 += d;
                e.2 += r;
                added += a;
                removed += d;
            }
        }
        Self {
            commits: commits
                .iter()
                .filter(|c| !c.merge)
                .map(|c| c.id.clone())
                .collect(),
            authors: people.len(),
            files_changed: paths.len(),
            added,
            removed,
            languages: sorted(langs),
        }
    }
}

/// Per-language totals, largest net change first.
fn sorted(langs: HashMap<String, (u64, u64, u64)>) -> Vec<LangDelta> {
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
    languages
}

impl Insights<'_> {
    /// Per-language change between two revisions given as commit ids, from the cached
    /// non-merge commits in `a..b`. Unknown ids give an empty comparison.
    pub fn compare(&self, base_id: &str, head_id: &str) -> Comparison {
        let src = self.source();
        // Walk parents by id: state is the visited set of the two ranges, never the history.
        let reach = |from: &str| -> HashSet<Rank> {
            let mut seen = HashSet::new();
            let mut stack: Vec<Rank> = src.commit_of(from).into_iter().collect();
            while let Some(r) = stack.pop() {
                if !seen.insert(r) {
                    continue;
                }
                stack.extend(src.parent_ids(r).iter().filter_map(|p| src.commit_of(p)));
            }
            seen
        };
        let base = reach(base_id);
        let mut range: Vec<Rank> = reach(head_id)
            .into_iter()
            .filter(|i| !base.contains(i))
            .collect();
        range.sort_unstable();
        let mut langs: HashMap<String, (u64, u64, u64)> = HashMap::new();
        let mut people = HashSet::new();
        let mut paths: HashSet<PathId> = HashSet::new();
        let (mut added, mut removed) = (0, 0);
        let mut who: std::collections::HashMap<Rank, usize> = std::collections::HashMap::new();
        let live: Vec<Rank> = range
            .iter()
            .copied()
            .filter(|r| {
                src.commit(*r).is_some_and(|c| {
                    who.insert(c.rank, self.person_of(c.ident));
                    !c.merge
                })
            })
            .collect();
        for (rank, rows) in src.rows_for(live) {
            people.insert(who.get(&rank).copied().unwrap_or(0));
            for row in &rows {
                let Some(l) = self.row_lines(row) else {
                    continue;
                };
                paths.insert(row.path);
                let name = src.path(row.path).unwrap_or_default();
                let e = langs.entry(language(&name)).or_default();
                e.0 += u64::from(l.add);
                e.1 += u64::from(l.del);
                e.2 += u64::from(l.rewritten);
                added += u64::from(l.add);
                removed += u64::from(l.del);
            }
        }
        let languages = sorted(langs);
        Comparison {
            commits: range
                .iter()
                .filter_map(|r| src.commit_id(*r).map(|s| s.to_string()))
                .collect(),
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
    use crate::insights::fixtures::chain;
    use crate::{Insights, Options};

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

    #[test]
    fn a_range_read_from_git_sums_like_the_index() {
        use super::{Comparison, RawCommit};
        let raw = |id: &str, who: &str, merge: bool, files: &[(&str, u64, u64)]| RawCommit {
            id: id.to_owned(),
            author: who.to_owned(),
            merge,
            files: files
                .iter()
                .map(|(p, a, d)| ((*p).to_owned(), *a, *d, 0))
                .collect(),
        };
        let c = Comparison::of_raw(&[
            raw("m", "x", true, &[("a.rs", 99, 99)]),
            raw("d", "x", false, &[("a.rs", 5, 1), ("b.py", 2, 0)]),
            raw("c", "y", false, &[("a.rs", 1, 0)]),
        ]);
        assert_eq!(c.commits, ["d", "c"], "merges are left out");
        assert_eq!(
            (c.authors, c.files_changed, c.added, c.removed),
            (2, 2, 8, 1)
        );
        assert_eq!(c.languages[0].language, "Rust");
        assert_eq!(c.languages[0].net, 5);
    }
}
