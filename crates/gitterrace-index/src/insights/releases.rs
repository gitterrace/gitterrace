//! Release rhythm: tags as releases, the gaps between them and the time since the last.

use std::sync::Arc;

use gitterrace_git::{RefInfo, RefKind};

use crate::feed::CommitRange;
use crate::insights::{CountContext, Counted, Insights};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Release {
    pub name: String,
    pub commit: String,
    pub time: i64,
    /// Commits since the previous release (first release: all commits up to it), counted by
    /// the one membership rule: merges out, bots by option, the selection applied.
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

/// `(name, commit id)` of every tag, as [`Insights::cadence_in`] takes them.
pub fn release_tags(refs: &[RefInfo]) -> Vec<(String, String)> {
    refs.iter()
        .filter(|r| r.kind == RefKind::Tag)
        .map(|r| (r.name.clone(), r.target.clone()))
        .collect()
}

impl Insights<'_> {
    /// Release rhythm from tags, counting the commits between releases as `c` counts them.
    /// Tags whose commit is not in the records are dropped. The patterns page passes an
    /// all-time `c`: releases are a whole-history view.
    pub fn cadence_in(&self, tags: &[(String, String)], c: &Counted<'_>) -> Cadence {
        // No tags, no releases: the count between them is never read.
        if tags.is_empty() {
            return self.cadence_with(tags, c.context(), |_| false);
        }
        // One flag per rank: a hash set of 345k ranks cost more than the count itself.
        let mut kept = vec![false; self.source().len()];
        for r in c.stream() {
            if let Some(k) = kept.get_mut(r.rank.0 as usize) {
                *k = true;
            }
        }
        self.cadence_with(tags, c.context(), |r| {
            kept.get(r.rank.0 as usize).copied().unwrap_or(false)
        })
    }

    fn cadence_with(
        &self,
        tags: &[(String, String)],
        cx: &CountContext,
        counts: impl Fn(&crate::feed::CommitRow) -> bool,
    ) -> Cadence {
        let src = self.source();
        // (rank, time, id, tag name) of every tag whose commit is indexed.
        let mut found: Vec<(usize, i64, Arc<str>, &str)> = tags
            .iter()
            .filter_map(|(n, id)| {
                let rank = src.commit_of(id)?;
                let c = src.commit(rank)?;
                Some((rank.0 as usize, c.time, src.commit_id(rank)?, n.as_str()))
            })
            .collect();
        found.sort_by_key(|(i, t, _, n)| (*t, *i, *n));
        // Counted commits up to and including each rank: the gap between two releases is the
        // difference, so it never counts a merge or a commit the selection leaves out.
        let mut upto: Vec<usize> = Vec::new();
        let scan = if found.is_empty() { None } else { Some(src.commits_in(CommitRange::All)) };
        for c in scan.into_iter().flatten() {
            let i = c.rank.0 as usize;
            if upto.len() <= i {
                upto.resize(i + 1, 0);
            }
            upto[i] = usize::from(counts(&c));
        }
        for i in 1..upto.len() {
            upto[i] += upto[i - 1];
        }
        let through = |i: usize| upto.get(i).or_else(|| upto.last()).copied().unwrap_or(0);
        let mut releases = Vec::new();
        let mut prev: Option<(usize, i64)> = None;
        for (i, time, id, name) in found {
            releases.push(Release {
                name: name.to_owned(),
                commit: id.to_string(),
                time,
                commits_since: prev
                    .map_or_else(|| through(i), |(p, _)| through(i).saturating_sub(through(p))),
                days_since: prev.map(|(_, pt)| cx.days_between(pt, time)),
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
            days_since_last: releases.last().map(|r| cx.days_between(r.time, self.now)),
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
        let c = ins.in_period(crate::Period::All, |c| ins.cadence_in(&tags, c));
        assert_eq!(c.releases.len(), 2);
        assert_eq!(c.releases[0].name, "v1");
        assert_eq!(c.releases[0].commits_since, 2);
        assert_eq!(c.releases[1].commits_since, 2);
        assert_eq!(c.releases[1].days_since, Some(30));
        assert_eq!(c.median_days_between, Some(30.0));
        assert_eq!(c.days_since_last, Some(0));
    }

    /// The commits between two releases are counted by the one rule: a merge and, with
    /// bots off, a bot commit are left out, and an author selection keeps only that author.
    #[test]
    fn commits_between_releases_follow_the_membership_rule() {
        use crate::feed::{FixedClock, MemorySource};
        use crate::insights::fixtures::{file, rec};
        use crate::{CountContext, Period, Scope, ScopeParams};
        let mut recs = vec![
            rec("a", "Ann", 0, vec![file("x.rs", 1, 0, 1)]),
            rec("b", "Bob", 1, vec![file("x.rs", 1, 0, 2)]),
            rec("m", "Ann", 2, vec![file("x.rs", 1, 0, 3)]),
            rec("bot", "dependabot[bot]", 3, vec![file("x.rs", 1, 0, 4)]),
            rec("c", "Ann", 4, vec![file("x.rs", 1, 0, 5)]),
        ];
        recs[2].merge = true;
        let tags = vec![("v1".to_owned(), "a".to_owned()), ("v2".to_owned(), "c".to_owned())];
        let ins = Insights::new(&recs, Options::default(), None);
        let since =
            |c: &super::Cadence| c.releases.iter().map(|r| r.commits_since).collect::<Vec<_>>();
        assert_eq!(
            since(&ins.in_period(Period::All, |c| ins.cadence_in(&tags, c))),
            [1, 2],
            "b and c, never the merge or the bot"
        );
        let cx = CountContext::new(&MemorySource::new(&recs), &FixedClock(i64::MAX));
        let mut p = ScopeParams::default();
        p.select.q = Some("author:ann@x.io".to_owned());
        let s = Scope::from_params(&p, Options::default(), Period::All).expect("scope");
        let c = ins.counted(&s, cx);
        assert_eq!(since(&ins.cadence_in(&tags, &c)), [1, 1], "Ann's own: a, then c");
    }
}
