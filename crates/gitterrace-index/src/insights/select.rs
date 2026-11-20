//! What a statistic is asked about, in the form every surface reads from a query string: the
//! commit selection (type, scope, author, path) and the counting options (bots, generated files,
//! ignore rules, whitespace, extra excludes). The web pages, the HTTP API and the MCP server all
//! parse these two structs, so the same URL parameters select the same commits everywhere.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, SourceError, Stream, SumOverflow, Sums,
    fold_author_day, fold_path_author,
};
use crate::insights::Options;
use crate::query::{Matcher, Query, QueryError};

/// Which commits count, as the URL says it: the filter text `q` alone.
/// [`Selection::query`] reads it into a [`Query`], the only form anything is selected by.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Selection {
    /// Filter text, `author:ann path:src !merge`: see [`crate::query::KEYWORDS`].
    pub q: Option<String>,
}

fn set(v: Option<&String>) -> Option<&str> {
    v.map(|s| s.trim()).filter(|s| !s.is_empty())
}

impl Selection {
    /// Type value of commits without a type.
    pub const OTHER: &str = "other";
    /// Scope value of commits without a scope.
    pub const NO_SCOPE: &str = "none";

    /// The selection of the filter text `q` alone.
    #[must_use]
    pub fn of(q: &Query) -> Self {
        Self {
            q: (!q.is_empty()).then(|| q.to_string()),
        }
    }

    /// The query `q` says. Empty or blank text is the empty query.
    ///
    /// # Errors
    /// [`QueryError`] when `q` does not parse.
    pub fn query(&self) -> Result<Query, QueryError> {
        Query::parse(set(self.q.as_ref()).unwrap_or_default())
    }
}

/// A [`Matcher`] and an optional author-time window applied to a source.
///
/// It keeps the commits
/// [`Matcher::apply`] plus a `[from, to)` time filter keep, renumbered to dense ranks, a path
/// selection trimming each commit to its rows under the prefix. State is one entry per kept
/// commit. Ids (paths, identities, labels) are the base's.
pub struct Scoped<'a> {
    base: Arc<dyn InsightsSource + 'a>,
    /// Per scoped rank: the base rank and, with a path selection, the kept row indices.
    kept: Vec<(Rank, Option<Vec<u32>>)>,
    by_base: BTreeMap<Rank, u32>,
    /// Set when stored sums added past u64, so the view reports the cache unreadable.
    over: SumOverflow,
}

impl<'a> Scoped<'a> {
    pub fn new(
        base: &Arc<dyn InsightsSource + 'a>,
        m: &Matcher,
        window: Option<&Range<i64>>,
    ) -> Self {
        let now = crate::feed::reference_time(&**base);
        let mut cand: Vec<Rank> = Vec::new();
        for c in base.commits_in(CommitRange::All) {
            if window.is_none_or(|w| w.contains(&c.time)) && m.commit_row(&**base, &c, now) {
                cand.push(c.rank);
            }
        }
        let kept: Vec<(Rank, Option<Vec<u32>>)> = if m.has_path() {
            base.rows_for(cand)
                .filter_map(|(r, rows)| {
                    let rows: Vec<u32> = rows
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| base.path(f.path).is_some_and(|n| m.row(&n)))
                        .map(|(i, _)| u32::try_from(i).unwrap_or(u32::MAX))
                        .collect();
                    (!rows.is_empty()).then_some((r, Some(rows)))
                })
                .collect()
        } else {
            cand.into_iter().map(|r| (r, None)).collect()
        };
        let by_base = kept
            .iter()
            .enumerate()
            .map(|(i, (r, _))| (*r, u32::try_from(i).unwrap_or(u32::MAX)))
            .collect();
        Self {
            base: Arc::clone(base),
            kept,
            by_base,
            over: SumOverflow::new(),
        }
    }

    fn base_rank(&self, r: Rank) -> Option<Rank> {
        self.kept.get(r.0 as usize).map(|k| k.0)
    }

    fn rows(&self, r: Rank) -> Vec<FileRow> {
        let Some((b, keep)) = self.kept.get(r.0 as usize) else {
            return Vec::new();
        };
        let rows = self
            .base
            .rows_for(vec![*b])
            .next()
            .map(|x| x.1)
            .unwrap_or_default();
        match keep {
            None => rows,
            Some(k) => k
                .iter()
                .filter_map(|i| rows.get(*i as usize).copied())
                .collect(),
        }
    }

    /// The base row restated for this view: scoped rank, sums over the kept rows.
    fn restate(&self, r: Rank, mut c: CommitRow) -> CommitRow {
        c.rank = r;
        if self.kept.get(r.0 as usize).is_some_and(|k| k.1.is_some()) {
            let rows = self.rows(r);
            c.files = u32::try_from(rows.len()).unwrap_or(u32::MAX);
            c.sums = Sums::default();
            for f in &rows {
                self.over.note(c.sums.add_row(f));
            }
        }
        c
    }
}

fn dense(i: usize) -> Rank {
    Rank(u32::try_from(i).unwrap_or(u32::MAX))
}

impl InsightsSource for Scoped<'_> {
    fn len(&self) -> usize {
        self.kept.len()
    }

    fn commit(&self, rank: Rank) -> Option<CommitRow> {
        let b = self.base_rank(rank)?;
        self.base.commit(b).map(|c| self.restate(rank, c))
    }

    fn commits_in(&self, range: CommitRange) -> Stream<'_, CommitRow> {
        let len = self.kept.len();
        let span = match &range {
            CommitRange::Ranks(r) => (r.start as usize).min(len)..(r.end as usize).min(len),
            _ => 0..len,
        };
        Box::new(span.filter_map(move |i| {
            let c = self.commit(dense(i))?;
            match &range {
                CommitRange::Time(t) => t.contains(&c.time).then_some(c),
                _ => Some(c),
            }
        }))
    }

    fn rows_for(&self, commits: Vec<Rank>) -> Stream<'_, (Rank, Vec<FileRow>)> {
        Box::new(
            commits
                .into_iter()
                .filter(|r| (r.0 as usize) < self.kept.len())
                .map(|r| (r, self.rows(r))),
        )
    }

    fn commit_id(&self, rank: Rank) -> Option<Arc<str>> {
        self.base.commit_id(self.base_rank(rank)?)
    }

    fn parent_ids(&self, rank: Rank) -> Vec<Arc<str>> {
        self.base_rank(rank)
            .map(|b| self.base.parent_ids(b))
            .unwrap_or_default()
    }

    fn find_commit(&self, id: &str) -> Result<Option<Rank>, SourceError> {
        let Some(b) = self.base.find_commit(id)? else {
            return Ok(None);
        };
        Ok(self.by_base.get(&b).map(|&i| Rank(i)))
    }

    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow> {
        fold_author_day(self.commits_in(CommitRange::All), &days, &self.over)
    }

    fn agg_path_author(&self, prefix: &str) -> Stream<'_, PathAuthorRow> {
        let prefix = prefix.trim_matches('/');
        fold_path_author(
            &self.over,
            |p| prefix.is_empty() || self.base.path(p).is_some_and(|n| under(&n, prefix)),
            |f| {
                for c in self.commits_in(CommitRange::All) {
                    f(&c, &self.rows(c.rank));
                }
            },
        )
    }

    fn postings(&self, key: Posting) -> Stream<'_, Hit> {
        Box::new((0..self.kept.len()).flat_map(move |i| {
            let r = dense(i);
            let hits: Vec<Hit> = match key {
                Posting::Author(a) => self
                    .commit(r)
                    .filter(|c| c.ident == a)
                    .map(|_| Hit { rank: r, row: 0 })
                    .into_iter()
                    .collect(),
                Posting::Path(p) => {
                    if self.commit(r).is_none_or(|c| c.merge) {
                        Vec::new()
                    } else {
                        self.rows(r)
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| f.path == p)
                            .map(|(row, _)| Hit {
                                rank: r,
                                row: u32::try_from(row).unwrap_or(u32::MAX),
                            })
                            .collect()
                    }
                }
            };
            hits
        }))
    }

    fn rename_edges(&self) -> Stream<'_, RenameEdge> {
        Box::new((0..self.kept.len()).flat_map(move |i| {
            let r = dense(i);
            let edges: Vec<RenameEdge> = self
                .rows(r)
                .iter()
                .enumerate()
                .filter_map(|(row, f)| {
                    f.old.map(|old| RenameEdge {
                        rank: r,
                        row: u32::try_from(row).unwrap_or(u32::MAX),
                        old,
                        new: f.path,
                    })
                })
                .collect();
            edges
        }))
    }

    fn path(&self, id: PathId) -> Option<Arc<str>> {
        self.base.path(id)
    }

    fn find_path(&self, path: &str) -> Result<Option<PathId>, SourceError> {
        self.base.find_path(path)
    }

    fn paths(&self) -> usize {
        self.base.paths()
    }

    fn ident(&self, id: IdentId) -> Option<(Arc<str>, Arc<str>)> {
        self.base.ident(id)
    }

    fn idents(&self) -> usize {
        self.base.idents()
    }

    fn label(&self, id: LabelId) -> Option<Arc<str>> {
        self.base.label(id)
    }

    fn find_label(&self, name: &str) -> Result<Option<LabelId>, SourceError> {
        self.base.find_label(name)
    }

    fn failure(&self) -> Option<SourceError> {
        self.base.failure().or_else(|| self.over.failure())
    }
}

/// True when `path` is `prefix` itself or inside it.
pub fn under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The counting options as URL parameters. Absent ones keep the defaults they are applied to.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct OptionParams {
    /// `1` counts bot accounts, `0` leaves them out.
    pub bots: Option<String>,
    /// `1` counts lock files and minified output, `0` leaves them out.
    pub generated: Option<String>,
    /// `0` counts paths that `.gitignore` rules out.
    pub ignore: Option<String>,
    /// `1` counts merge commits as commits, never their lines or files.
    pub merges: Option<String>,
    /// `1` counts whitespace-only changes.
    pub ws: Option<String>,
    /// Comma separated globs left out on top of the configured ones.
    pub exclude: Option<String>,
}

impl OptionParams {
    /// The URL's own globs, without the configured ones.
    pub fn own_exclude(&self) -> Vec<String> {
        self.exclude
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// `defaults` (the configuration) overridden by what the URL says.
    pub fn apply(&self, defaults: Options) -> Options {
        let flag = |v: Option<&String>, default: bool| match v.map(String::as_str) {
            Some("1") => true,
            Some("0") => false,
            _ => default,
        };
        let mut exclude = defaults.exclude;
        exclude.extend(self.own_exclude());
        Options {
            exclude,
            include_generated: flag(self.generated.as_ref(), defaults.include_generated),
            respect_ignore: flag(self.ignore.as_ref(), defaults.respect_ignore),
            count_whitespace: flag(self.ws.as_ref(), defaults.count_whitespace),
            include_bots: flag(self.bots.as_ref(), defaults.include_bots),
            include_merges: flag(self.merges.as_ref(), defaults.include_merges),
        }
    }
}

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;
