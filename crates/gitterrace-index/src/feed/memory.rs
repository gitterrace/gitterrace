//! [`InsightsSource`] over today's in-memory records: the oracle side of the equivalence guard,
//! and the source pages are ported against before the segment reader exists. String and id
//! tables are built once on first use. Aggregates are folded per call (this side has the
//! records resident anyway, its memory is not the goal).

use rustc_hash::FxHashMap as FastMap;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use crate::CommitRecord;
use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, SourceError, Stream, Sums,
};
use crate::insights::{local_day, under};

/// Interned strings and per-record ids, in first-use order over the records.
#[derive(Debug, Default)]
pub(crate) struct MemoryTables {
    paths: Vec<Arc<str>>,
    path_ids: FastMap<Arc<str>, u32>,
    /// Raw (name, email) as written, the key of an identity id, so a mailmap
    /// edit changes strings only, never ids.
    idents: Vec<(Arc<str>, Arc<str>)>,
    ident_ids: FastMap<(Arc<str>, Arc<str>), u32>,
    /// Shown strings per identity id, the mailmap applied. Equal to `idents` without a map.
    shown: Vec<(Arc<str>, Arc<str>)>,
    labels: Vec<Arc<str>>,
    label_ids: FastMap<Arc<str>, u32>,
    commit_ids: FastMap<Arc<str>, u32>,
    /// Per path id: non-merge (rank, row) hits, ascending.
    by_path: Vec<Vec<Hit>>,
    /// Per record: identity, kind, scope.
    per_commit: Vec<(IdentId, Option<LabelId>, Option<LabelId>)>,
    /// Per record: sums over its rows, so `commit` is O(1).
    sums: Vec<Sums>,
    /// File rows of every record, flat, record `i` at `starts[i]..starts[i + 1]`.
    rows: Vec<FileRow>,
    starts: Vec<usize>,
}

/// Raw (name, email) to the shown pair: the mailmap of the shown revision.
pub type IdentPairs = std::collections::HashMap<(String, String), (String, String)>;

fn intern(list: &mut Vec<Arc<str>>, ids: &mut FastMap<Arc<str>, u32>, s: &str) -> u32 {
    if let Some(&i) = ids.get(s) {
        return i;
    }
    let a: Arc<str> = s.into();
    let i = u32::try_from(list.len()).unwrap_or(u32::MAX);
    list.push(Arc::clone(&a));
    ids.insert(a, i);
    i
}

impl MemoryTables {
    fn build(records: &[CommitRecord], map: Option<&IdentPairs>) -> Self {
        let mut t = Self::default();
        t.starts.push(0);
        for (i, r) in records.iter().enumerate() {
            let key: (Arc<str>, Arc<str>) = (r.author.as_str().into(), r.email.as_str().into());
            let ident = if let Some(&id) = t.ident_ids.get(&key) {
                id
            } else {
                let id = u32::try_from(t.idents.len()).unwrap_or(u32::MAX);
                let shown = map
                    .and_then(|m| m.get(&(r.author.clone(), r.email.clone())))
                    .map_or_else(
                        || key.clone(),
                        |(n, e)| (n.as_str().into(), e.as_str().into()),
                    );
                t.shown.push(shown);
                t.idents.push(key.clone());
                t.ident_ids.insert(key, id);
                id
            };
            let kind = r
                .kind
                .as_deref()
                .map(|k| LabelId(intern(&mut t.labels, &mut t.label_ids, k)));
            let scope = r
                .scope
                .as_deref()
                .map(|s| LabelId(intern(&mut t.labels, &mut t.label_ids, s)));
            t.per_commit.push((IdentId(ident), kind, scope));
            let mut sums = Sums::default();
            for (fi, f) in r.files.iter().enumerate() {
                let p = intern(&mut t.paths, &mut t.path_ids, &f.path);
                let old = f
                    .old
                    .as_deref()
                    .map(|o| PathId(intern(&mut t.paths, &mut t.path_ids, o)));
                t.by_path.resize_with(t.paths.len(), Vec::new);
                if !r.merge {
                    t.by_path[p as usize].push(Hit {
                        rank: rank(i),
                        row: u32::try_from(fi).unwrap_or(u32::MAX),
                    });
                }
                let row = FileRow {
                    path: PathId(p),
                    old,
                    add: f.additions,
                    del: f.deletions,
                    lines: f.lines,
                    rewritten: f.rewritten,
                    code: f.code.map(|c| (c.additions, c.deletions, c.rewritten)),
                    binary: f.binary,
                    deleted: f.deleted,
                };
                let added = sums.add_row(&row);
                // Records, not stored bytes: u32 counts per row cannot sum past u64.
                debug_assert!(added.is_some(), "record counts summed past u64");
                t.rows.push(row);
            }
            t.sums.push(sums);
            t.starts.push(t.rows.len());
            t.commit_ids
                .entry(r.id.as_str().into())
                .or_insert_with(|| u32::try_from(i).unwrap_or(u32::MAX));
        }
        t
    }

    fn files(&self, i: usize) -> &[FileRow] {
        &self.rows[self.starts[i]..self.starts[i + 1]]
    }
}

/// The memory adapter. Rank `i` is record `i` (records are oldest first).
pub struct MemorySource<'a> {
    records: Held<'a>,
    tables: OnceLock<MemoryTables>,
    map: Option<IdentPairs>,
}

enum Held<'a> {
    Borrowed(&'a [CommitRecord]),
    Shared(Arc<Vec<CommitRecord>>),
}

impl<'a> MemorySource<'a> {
    pub const fn new(records: &'a [CommitRecord]) -> Self {
        Self {
            records: Held::Borrowed(records),
            tables: OnceLock::new(),
            map: None,
        }
    }

    fn recs(&self) -> &[CommitRecord] {
        match &self.records {
            Held::Borrowed(r) => r,
            Held::Shared(r) => r,
        }
    }

    fn t(&self) -> &MemoryTables {
        self.tables
            .get_or_init(|| MemoryTables::build(self.recs(), self.map.as_ref()))
    }
}

impl MemorySource<'static> {
    /// Owns its records, so it can live in an `Arc<dyn InsightsSource>` cache.
    pub const fn shared(records: Arc<Vec<CommitRecord>>) -> Self {
        Self {
            records: Held::Shared(records),
            tables: OnceLock::new(),
            map: None,
        }
    }

    /// Records with raw identities (as stored) plus the shown revision's mailmap: ids key the
    /// raw pair, strings come mapped. The same identity rule as
    /// [`crate::feed::DiskSource`], so the oracle compares like with like.
    pub const fn shared_raw(records: Arc<Vec<CommitRecord>>, map: IdentPairs) -> Self {
        Self {
            records: Held::Shared(records),
            tables: OnceLock::new(),
            map: Some(map),
        }
    }
}

impl MemorySource<'_> {
    fn row(&self, i: usize) -> CommitRow {
        let r = &self.recs()[i];
        let t = self.t();
        let (ident, kind, scope) = t.per_commit[i];
        CommitRow {
            rank: rank(i),
            ident,
            time: r.time,
            offset: r.offset,
            merge: r.merge,
            kind,
            scope,
            files: u32::try_from(r.files.len()).unwrap_or(u32::MAX),
            sums: t.sums[i],
        }
    }
}

fn rank(i: usize) -> Rank {
    Rank(u32::try_from(i).unwrap_or(u32::MAX))
}

impl InsightsSource for MemorySource<'_> {
    fn len(&self) -> usize {
        self.recs().len()
    }

    fn commit(&self, rank: Rank) -> Option<CommitRow> {
        let i = rank.0 as usize;
        (i < self.recs().len()).then(|| self.row(i))
    }

    fn commits_in(&self, range: CommitRange) -> Stream<'_, CommitRow> {
        let n = self.recs().len();
        match range {
            CommitRange::All => Box::new((0..n).map(|i| self.row(i))),
            CommitRange::Ranks(r) => {
                let (a, b) = ((r.start as usize).min(n), (r.end as usize).min(n));
                Box::new((a..b).map(|i| self.row(i)))
            }
            CommitRange::Time(t) => Box::new(
                (0..n)
                    .filter(move |&i| t.contains(&self.recs()[i].time))
                    .map(|i| self.row(i)),
            ),
        }
    }

    fn rows_for(&self, commits: Vec<Rank>) -> Stream<'_, (Rank, Vec<FileRow>)> {
        Box::new(commits.into_iter().filter_map(|r| {
            let i = r.0 as usize;
            (i < self.recs().len()).then(|| (r, self.t().files(i).to_vec()))
        }))
    }

    fn commit_id(&self, rank: Rank) -> Option<Arc<str>> {
        self.recs()
            .get(rank.0 as usize)
            .map(|r| r.id.as_str().into())
    }

    fn parent_ids(&self, rank: Rank) -> Vec<Arc<str>> {
        self.recs()
            .get(rank.0 as usize)
            .map(|r| r.parents.iter().map(|p| p.as_str().into()).collect())
            .unwrap_or_default()
    }

    fn find_commit(&self, id: &str) -> Result<Option<Rank>, SourceError> {
        Ok(self.t().commit_ids.get(id).map(|&i| Rank(i)))
    }

    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow> {
        let mut m: BTreeMap<(i64, IdentId), (u64, Sums)> = BTreeMap::new();
        for (i, r) in self.recs().iter().enumerate() {
            let day = local_day(r);
            if r.merge || !days.contains(&day) {
                continue;
            }
            let e = m.entry((day, self.t().per_commit[i].0)).or_default();
            e.0 += 1;
            let added = e.1.merge(&self.t().sums[i]);
            debug_assert!(added.is_some(), "record counts summed past u64");
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
        for (i, r) in self.recs().iter().enumerate().filter(|(_, r)| !r.merge) {
            for (f, row) in r.files.iter().zip(self.t().files(i)) {
                if !(prefix.is_empty() || under(&f.path, prefix)) {
                    continue;
                }
                let e = m.entry((row.path, self.t().per_commit[i].0)).or_default();
                e.0 += 1;
                let added = e.1.add_row(row);
                debug_assert!(added.is_some(), "record counts summed past u64");
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
        match key {
            Posting::Path(p) => Box::new(
                self.t()
                    .by_path
                    .get(p.0 as usize)
                    .into_iter()
                    .flatten()
                    .copied(),
            ),
            Posting::Author(a) => Box::new(
                (0..self.recs().len())
                    .filter(move |&i| self.t().per_commit[i].0 == a)
                    .map(|i| Hit {
                        rank: rank(i),
                        row: 0,
                    }),
            ),
        }
    }

    fn rename_edges(&self) -> Stream<'_, RenameEdge> {
        Box::new((0..self.recs().len()).flat_map(move |i| {
            let edges: Vec<RenameEdge> = self
                .t()
                .files(i)
                .iter()
                .enumerate()
                .filter_map(|(fi, f)| {
                    f.old.map(|old| RenameEdge {
                        rank: rank(i),
                        row: u32::try_from(fi).unwrap_or(u32::MAX),
                        old,
                        new: f.path,
                    })
                })
                .collect();
            edges
        }))
    }

    fn path(&self, id: PathId) -> Option<Arc<str>> {
        self.t().paths.get(id.0 as usize).cloned()
    }

    fn find_path(&self, path: &str) -> Result<Option<PathId>, SourceError> {
        Ok(self.t().path_ids.get(path).map(|&i| PathId(i)))
    }

    fn paths(&self) -> usize {
        self.t().paths.len()
    }

    fn ident(&self, id: IdentId) -> Option<(Arc<str>, Arc<str>)> {
        self.t().shown.get(id.0 as usize).cloned()
    }

    fn idents(&self) -> usize {
        self.t().idents.len()
    }

    fn label(&self, id: LabelId) -> Option<Arc<str>> {
        self.t().labels.get(id.0 as usize).cloned()
    }

    fn find_label(&self, name: &str) -> Result<Option<LabelId>, SourceError> {
        Ok(self.t().label_ids.get(name).map(|&i| LabelId(i)))
    }

    fn failure(&self) -> Option<SourceError> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::MemorySource;
    use crate::feed::{CommitRange, InsightsSource, Posting, Rank};
    use crate::insights::fixtures::{chain, file};

    #[test]
    fn commits_rows_and_strings_round_trip() {
        let mut recs = chain();
        recs[3].files.push(crate::FileStat {
            old: Some("x.rs".into()),
            ..file("z.rs", 1, 0, 1)
        });
        let s = MemorySource::new(&recs);
        assert_eq!(s.len(), 4);
        let c = s.commit(Rank(2)).expect("commit 2");
        assert_eq!((c.sums.add, c.sums.del, c.files), (300, 103, 2));
        assert_eq!(
            s.ident(c.ident).map(|i| i.0.to_string()).as_deref(),
            Some("Bob")
        );
        assert_eq!(s.find_commit("c"), Ok(Some(Rank(2))));
        assert_eq!(s.commits_in(CommitRange::Ranks(1..3)).count(), 2);
        let x = s.path_id("x.rs").expect("x.rs");
        let hits: Vec<u32> = s.postings(Posting::Path(x)).map(|h| h.rank.0).collect();
        assert_eq!(hits, [0, 1, 2]);
        let e: Vec<_> = s.rename_edges().collect();
        assert_eq!(e.len(), 1);
        assert_eq!(s.path(e[0].old).as_deref(), Some("x.rs"));
        let days: u64 = s
            .agg_author_day(i64::MIN..i64::MAX)
            .map(|r| r.commits)
            .sum();
        assert_eq!(days, 4);
        let owned: u64 = s.agg_path_author("").map(|r| r.touches).sum();
        assert_eq!(owned, 6);
        let shared = MemorySource::shared(std::sync::Arc::new(recs.clone()));
        assert_eq!(shared.find_commit("c"), Ok(Some(Rank(2))));
    }
}
