//! Release rhythm: tags as releases, the gaps between them and the time since the last.

use std::sync::Arc;

use gitterrace_git::{RefInfo, RefKind};

use crate::insights::Insights;

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

/// `(name, commit id)` of every tag, as [`Insights::cadence`] takes them.
pub fn release_tags(refs: &[RefInfo]) -> Vec<(String, String)> {
    refs.iter()
        .filter(|r| r.kind == RefKind::Tag)
        .map(|r| (r.name.clone(), r.target.clone()))
        .collect()
}

impl Insights<'_> {
    /// Release rhythm from tags. Tags whose commit is not in the records are dropped.
    pub fn cadence(&self, tags: &[(String, String)]) -> Cadence {
        let src = self.source();
        // (rank, time, id, tag name) of every tag whose commit is indexed.
        let mut found: Vec<(usize, i64, Arc<str>, &str)> = tags
            .iter()
            .filter_map(|(n, id)| {
                let rank = src.find_commit(id)?;
                let c = src.commit(rank)?;
                Some((rank.0 as usize, c.time, src.commit_id(rank)?, n.as_str()))
            })
            .collect();
        found.sort_by_key(|(i, t, _, n)| (*t, *i, *n));
        let mut releases = Vec::new();
        let mut prev: Option<(usize, i64)> = None;
        for (i, time, id, name) in found {
            releases.push(Release {
                name: name.to_owned(),
                commit: id.to_string(),
                time,
                commits_since: prev.map_or(i + 1, |(p, _)| i.saturating_sub(p)),
                days_since: prev.map(|(_, pt)| time.saturating_sub(pt).div_euclid(86_400)),
            });
            prev = Some((i, time));
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
                .map(|r| self.now.saturating_sub(r.time).div_euclid(86_400)),
            median_days_between: median,
            releases,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::insights::fixtures::chain;
    use crate::{Insights, Options};

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
}
