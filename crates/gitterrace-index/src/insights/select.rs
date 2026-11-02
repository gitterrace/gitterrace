//! What a statistic is asked about, in the form every surface reads from a query string: the
//! commit selection (type, scope, author, path) and the counting options (bots, generated files,
//! ignore rules, whitespace, extra excludes). The web pages, the HTTP API and the MCP server all
//! parse these two structs, so the same URL parameters select the same commits everywhere.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::CommitRecord;
use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, Stream, Sums, local_day_of,
};
use crate::insights::Options;

/// Which commits count. Empty strings count as unset.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Selection {
    /// Conventional-commit type; [`Selection::OTHER`] selects commits without one.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Conventional-commit scope; [`Selection::NO_SCOPE`] selects commits without one.
    pub scope: Option<String>,
    /// E-mail (any case) or exact author name.
    pub author: Option<String>,
    /// Path prefix: a kept commit lists only its files under the prefix.
    pub path: Option<String>,
}

fn set(v: Option<&String>) -> Option<&str> {
    v.map(String::as_str).filter(|s| !s.is_empty())
}

impl Selection {
    /// Type value of commits without a type.
    pub const OTHER: &str = "other";
    /// Scope value of commits without a scope.
    pub const NO_SCOPE: &str = "none";

    pub fn kind(&self) -> Option<&str> {
        set(self.kind.as_ref())
    }

    pub fn scope(&self) -> Option<&str> {
        set(self.scope.as_ref())
    }

    pub fn author(&self) -> Option<&str> {
        set(self.author.as_ref())
    }

    /// Path prefix without surrounding slashes, `None` for the whole tree.
    pub fn path(&self) -> Option<&str> {
        set(self.path.as_ref())
            .map(|p| p.trim_matches('/'))
            .filter(|p| !p.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.kind().is_none()
            && self.scope().is_none()
            && self.author().is_none()
            && self.path().is_none()
    }

    fn matches(&self, r: &CommitRecord) -> bool {
        self.kind()
            .is_none_or(|k| r.kind.as_deref().unwrap_or(Self::OTHER) == k)
            && self
                .scope()
                .is_none_or(|s| r.scope.as_deref().unwrap_or(Self::NO_SCOPE) == s)
            && self
                .author()
                .is_none_or(|a| r.email.eq_ignore_ascii_case(a) || r.author == a)
    }

    /// The records the selection keeps. With a path each kept commit lists only the files under
    /// it, so line counts and churn are the path's own. Without a selection the input is borrowed.
    pub fn apply<'a>(&self, records: &'a [CommitRecord]) -> Cow<'a, [CommitRecord]> {
        if self.is_empty() {
            return Cow::Borrowed(records);
        }
        let path = self.path();
        let out = records
            .iter()
            .filter(|r| self.matches(r))
            .filter_map(|r| {
                path.map_or_else(
                    || Some(r.clone()),
                    |p| {
                        let files: Vec<_> = r
                            .files
                            .iter()
                            .filter(|f| under(&f.path, p))
                            .cloned()
                            .collect();
                        (!files.is_empty()).then(|| CommitRecord { files, ..r.clone() })
                    },
                )
            })
            .collect();
        Cow::Owned(out)
    }
}

/// A [`Selection`] and an optional author-time window applied to a source.
///
/// It keeps the commits
/// [`Selection::apply`] plus a `[from, to)` time filter keep, renumbered to dense ranks, a path
/// selection trimming each commit to its rows under the prefix. State is one entry per kept
/// commit. Ids (paths, identities, labels) are the base's.
pub struct Scoped<'a> {
    base: Arc<dyn InsightsSource + 'a>,
    /// Per scoped rank: the base rank and, with a path selection, the kept row indices.
    kept: Vec<(Rank, Option<Vec<u32>>)>,
    by_base: BTreeMap<Rank, u32>,
}

impl<'a> Scoped<'a> {
    pub fn new(
        base: Arc<dyn InsightsSource + 'a>,
        sel: &Selection,
        window: Option<Range<i64>>,
    ) -> Self {
        let label_is = |id: Option<LabelId>, want: &str, none: &str| {
            id.and_then(|l| base.label(l)).as_deref().unwrap_or(none) == want
        };
        let mut cand: Vec<Rank> = Vec::new();
        for c in base.commits_in(CommitRange::All) {
            let keep = sel
                .kind()
                .is_none_or(|k| label_is(c.kind, k, Selection::OTHER))
                && sel
                    .scope()
                    .is_none_or(|s| label_is(c.scope, s, Selection::NO_SCOPE))
                && sel.author().is_none_or(|a| {
                    base.ident(c.ident)
                        .is_some_and(|(name, email)| email.eq_ignore_ascii_case(a) || &*name == a)
                })
                && window.as_ref().is_none_or(|w| w.contains(&c.time));
            if keep {
                cand.push(c.rank);
            }
        }
        let kept: Vec<(Rank, Option<Vec<u32>>)> = match sel.path() {
            None => cand.into_iter().map(|r| (r, None)).collect(),
            Some(p) => base
                .rows_for(cand)
                .filter_map(|(r, rows)| {
                    let rows: Vec<u32> = rows
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| base.path(f.path).is_some_and(|n| under(&n, p)))
                        .map(|(i, _)| u32::try_from(i).unwrap_or(u32::MAX))
                        .collect();
                    (!rows.is_empty()).then_some((r, Some(rows)))
                })
                .collect(),
        };
        let by_base = kept
            .iter()
            .enumerate()
            .map(|(i, (r, _))| (*r, u32::try_from(i).unwrap_or(u32::MAX)))
            .collect();
        Self {
            base,
            kept,
            by_base,
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
                c.sums.add_row(f);
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

    fn find_commit(&self, id: &str) -> Option<Rank> {
        let b = self.base.find_commit(id)?;
        self.by_base.get(&b).map(|&i| Rank(i))
    }

    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow> {
        let mut m: BTreeMap<(i64, IdentId), (u64, Sums)> = BTreeMap::new();
        for c in self.commits_in(CommitRange::All).filter(|c| !c.merge) {
            let day = local_day_of(c.time, c.offset);
            if days.contains(&day) {
                let e = m.entry((day, c.ident)).or_default();
                e.0 += 1;
                e.1.merge(&c.sums);
            }
        }
        Box::new(
            m.into_iter()
                .map(|((day, ident), (commits, sums))| AuthorDayRow {
                    day,
                    ident,
                    commits,
                    sums,
                }),
        )
    }

    fn agg_path_author(&self, prefix: &str) -> Stream<'_, PathAuthorRow> {
        let prefix = prefix.trim_matches('/');
        let mut m: BTreeMap<(PathId, IdentId), (u64, Sums)> = BTreeMap::new();
        for c in self.commits_in(CommitRange::All).filter(|c| !c.merge) {
            for f in self.rows(c.rank) {
                if prefix.is_empty() || self.base.path(f.path).is_some_and(|n| under(&n, prefix)) {
                    let e = m.entry((f.path, c.ident)).or_default();
                    e.0 += 1;
                    e.1.add_row(&f);
                }
            }
        }
        Box::new(
            m.into_iter()
                .map(|((path, ident), (touches, sums))| PathAuthorRow {
                    path,
                    ident,
                    touches,
                    sums,
                }),
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

    fn find_path(&self, path: &str) -> Option<PathId> {
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

    fn find_label(&self, name: &str) -> Option<LabelId> {
        self.base.find_label(name)
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
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{OptionParams, Scoped, Selection, under};
    use crate::feed::{CommitRange, InsightsSource, MemorySource, Rank};
    use crate::insights::fixtures::{file, rec};
    use crate::{CommitRecord, Options};

    fn sample() -> Vec<CommitRecord> {
        let mut v = vec![
            rec("a", "ann", 1, vec![file("crates/web/a.rs", 3, 0, 3)]),
            rec("b", "bob", 50, vec![file("crates/git/b.rs", 3, 0, 3)]),
            rec("c", "ann", 99, vec![file("crates/web/c.rs", 3, 0, 3)]),
            rec("d", "ann", 100, vec![file("crates/webby/d.rs", 3, 0, 3)]),
        ];
        for (r, (k, s)) in v.iter_mut().zip([
            (Some("fix"), Some("web")),
            (Some("feat"), None),
            (None, None),
            (Some("fix"), Some("git")),
        ]) {
            r.kind = k.map(Into::into);
            r.scope = s.map(Into::into);
            r.files.push(file("other/x.rs", 1, 0, 1));
        }
        v
    }

    fn ids(s: &Selection) -> Vec<String> {
        s.apply(&sample()).iter().map(|r| r.id.clone()).collect()
    }

    fn sel(kind: Option<&str>, scope: Option<&str>, author: Option<&str>) -> Selection {
        Selection {
            kind: kind.map(Into::into),
            scope: scope.map(Into::into),
            author: author.map(Into::into),
            path: None,
        }
    }

    #[test]
    fn selections_stack() {
        assert_eq!(ids(&Selection::default()), ["a", "b", "c", "d"]);
        assert_eq!(ids(&sel(Some("fix"), None, None)), ["a", "d"]);
        assert_eq!(ids(&sel(Some("fix"), Some("web"), None)), ["a"]);
        assert_eq!(ids(&sel(Some("other"), None, None)), ["c"]);
        assert_eq!(ids(&sel(None, Some("none"), None)), ["b", "c"]);
        assert_eq!(ids(&sel(Some("fix"), None, Some("ANN@x.io"))), ["a", "d"]);
        assert_eq!(ids(&sel(Some(""), None, None)), ["a", "b", "c", "d"]);
    }

    #[test]
    fn path_keeps_only_files_under_the_prefix() {
        let s = Selection {
            path: Some("/crates/web/".into()),
            ..Selection::default()
        };
        let out = s.apply(&sample()).into_owned();
        assert_eq!(
            out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "c"]
        );
        assert!(out.iter().all(|r| r.files.len() == 1));
        assert!(
            under("crates/web/a.rs", "crates/web") && !under("crates/webby/d.rs", "crates/web")
        );
    }

    #[test]
    fn url_options_override_the_defaults() {
        let defaults = Options {
            exclude: vec!["docs/*".into()],
            include_bots: true,
            ..Options::default()
        };
        let p = OptionParams {
            bots: Some("0".into()),
            ignore: Some("0".into()),
            exclude: Some(" *.md, ,x/*".into()),
            ..OptionParams::default()
        };
        let o = p.apply(defaults.clone());
        assert!(!o.include_bots && !o.respect_ignore && !o.include_generated);
        assert_eq!(o.exclude, ["docs/*", "*.md", "x/*"]);
        assert_eq!(OptionParams::default().apply(defaults.clone()), defaults);
    }

    /// The scoped view must keep exactly the commits and rows `Selection::apply` plus the
    /// window keep, in order, with sums over the kept rows. Every selection of
    /// `selections_stack` plus path and window cells.
    #[test]
    fn scoped_equals_apply() {
        let recs = sample();
        let base: Arc<dyn InsightsSource> = Arc::new(MemorySource::shared(Arc::new(recs.clone())));
        let cells = [
            Selection::default(),
            sel(Some("fix"), None, None),
            sel(Some("fix"), Some("web"), None),
            sel(Some("other"), None, None),
            sel(None, Some("none"), None),
            sel(Some("fix"), None, Some("ANN@x.io")),
            sel(None, None, Some("bob")),
            Selection {
                path: Some("/crates/web/".into()),
                ..Selection::default()
            },
        ];
        let t = |d: i64| crate::insights::fixtures::MON + d * crate::insights::fixtures::DAYS;
        for s in &cells {
            for w in [None, Some(t(40)..t(100)), Some(t(0)..t(1))] {
                let want: Vec<_> = s
                    .apply(&recs)
                    .iter()
                    .filter(|r| w.as_ref().is_none_or(|w| w.contains(&r.time)))
                    .cloned()
                    .collect();
                let v = Scoped::new(Arc::clone(&base), s, w.clone());
                assert_eq!(v.len(), want.len(), "{s:?} {w:?}");
                let rows: Vec<_> = v.commits_in(CommitRange::All).collect();
                for (i, (c, r)) in rows.iter().zip(&want).enumerate() {
                    assert_eq!(c.rank, Rank(u32::try_from(i).unwrap_or(0)));
                    assert_eq!(v.commit_id(c.rank).as_deref(), Some(r.id.as_str()));
                    assert_eq!(c.files as usize, r.files.len(), "{s:?} {}", r.id);
                    let add: u64 = r.files.iter().map(|f| u64::from(f.additions)).sum();
                    assert_eq!(c.sums.add, add);
                    assert_eq!(v.find_commit(&r.id), Some(c.rank));
                }
                assert_eq!(v.len(), want.len());
            }
        }
    }
}
