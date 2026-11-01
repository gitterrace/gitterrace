//! The `a..b` comparison: commits, people, files and per-language change between two revisions.

use std::collections::{HashMap, HashSet};

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

impl Insights<'_> {
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
            for (file, l) in self.files_of(i) {
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
}
