//! The `InsightsSource` trait over the disk base.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::feed::disk::record::{hash32, hex};
use crate::feed::disk::{DiskSource, SCAN_FROM, scan};
use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, Stream, Sums,
};

impl InsightsSource for DiskSource {
    fn len(&self) -> usize {
        self.len as usize
    }

    fn commit(&self, rank: Rank) -> Option<CommitRow> {
        if rank.0 >= self.len {
            return None;
        }
        self.ok(self.row(rank))
    }

    fn commits_in(&self, range: CommitRange) -> Stream<'_, CommitRow> {
        let n = self.len;
        let (ranks, time) = match range {
            CommitRange::All => (0..n, None),
            CommitRange::Ranks(r) => (r.start.min(n)..r.end.min(n), None),
            CommitRange::Time(t) => (0..n, Some(t)),
        };
        Box::new(
            scan::Scan::new(self, ranks.start, ranks.end, false)
                .filter(move |c| time.as_ref().is_none_or(|t| t.contains(&c.time))),
        )
    }

    fn rows_for(&self, commits: Vec<Rank>) -> Stream<'_, (Rank, Vec<FileRow>)> {
        // Many ascending ranks that cover much of their span: one sequential pass beats a cache
        // probe per commit. A sparse list (a few commits spread over history) would read the
        // whole span, so it keeps the lookups.
        let dense = |c: &[Rank]| {
            let span = u64::from(c[c.len() - 1].0 - c[0].0) + 1;
            (c.len() as u64).saturating_mul(4) >= span
        };
        if commits.len() > SCAN_FROM && commits.is_sorted() && dense(&commits) {
            let (from, to) = (commits[0].0, commits[commits.len() - 1].0 + 1);
            let mut s = scan::Scan::new(self, from, to, true);
            let mut want = commits.into_iter().peekable();
            return Box::new(std::iter::from_fn(move || {
                let mut files = Vec::new();
                loop {
                    let next = *want.peek()?;
                    let c = s.next_row(&mut files)?;
                    if c.rank == next {
                        want.next();
                        return Some((c.rank, std::mem::take(&mut files)));
                    }
                }
            }));
        }
        Box::new(commits.into_iter().filter_map(move |r| {
            if r.0 >= self.len {
                return None;
            }
            let mut files = Vec::new();
            self.ok(self.row_with(r, Some(&mut files)))
                .map(|_| (r, files))
        }))
    }

    fn commit_id(&self, rank: Rank) -> Option<Arc<str>> {
        if rank.0 >= self.len {
            return None;
        }
        self.ok(self.decode(rank, None))
            .map(|r| Arc::from(hex(&r.oid)))
    }

    fn parent_ids(&self, rank: Rank) -> Vec<Arc<str>> {
        if rank.0 >= self.len {
            return Vec::new();
        }
        self.ok(self.decode(rank, None))
            .map(|r| r.parents.iter().map(|p| Arc::from(hex(p))).collect())
            .unwrap_or_default()
    }

    fn find_commit(&self, id: &str) -> Option<Rank> {
        let oid = crate::store::oid_of(id)?;
        self.ok(self.rank_of(&oid)).flatten()
    }

    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow> {
        let mut m: BTreeMap<(i64, IdentId), (u64, Sums)> = BTreeMap::new();
        self.each_row(|c, _| {
            let day = crate::feed::local_day_of(c.time, c.offset);
            if c.merge || !days.contains(&day) {
                return;
            }
            let e = m.entry((day, c.ident)).or_default();
            e.0 += 1;
            e.1.merge(&c.sums);
        });
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
        let mut keep: BTreeMap<PathId, bool> = BTreeMap::new();
        let mut m: BTreeMap<(PathId, IdentId), (u64, Sums)> = BTreeMap::new();
        self.each_row(|c, files| {
            if c.merge {
                return;
            }
            for f in files {
                let ok = *keep.entry(f.path).or_insert_with(|| {
                    prefix.is_empty()
                        || self
                            .path(f.path)
                            .is_some_and(|p| crate::insights::under(&p, prefix))
                });
                if ok {
                    let e = m.entry((f.path, c.ident)).or_default();
                    e.0 += 1;
                    e.1.add_row(f);
                }
            }
        });
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
        let mut out = Vec::new();
        self.each_row(|c, files| match key {
            Posting::Path(p) if !c.merge => {
                for (i, f) in files.iter().enumerate() {
                    if f.path == p {
                        out.push(Hit {
                            rank: c.rank,
                            row: u32::try_from(i).unwrap_or(u32::MAX),
                        });
                    }
                }
            }
            Posting::Author(a) if c.ident == a => out.push(Hit {
                rank: c.rank,
                row: 0,
            }),
            _ => {}
        });
        Box::new(out.into_iter())
    }

    fn rename_edges(&self) -> Stream<'_, RenameEdge> {
        let mut out = Vec::new();
        self.each_row(|c, files| {
            for (i, f) in files.iter().enumerate() {
                if let Some(old) = f.old {
                    out.push(RenameEdge {
                        rank: c.rank,
                        row: u32::try_from(i).unwrap_or(u32::MAX),
                        old,
                        new: f.path,
                    });
                }
            }
        });
        Box::new(out.into_iter())
    }

    fn path(&self, id: PathId) -> Option<Arc<str>> {
        if id.0 >= self.counts[0] {
            return None;
        }
        self.ok(self.string(0, id.0))
    }

    fn find_path(&self, path: &str) -> Option<PathId> {
        let idx = if let Some(i) = self.path_hash.get() {
            i
        } else {
            let built = self.ok(self.path_index())?;
            self.path_hash.get_or_init(|| built)
        };
        let h = hash32(path);
        let start = idx.partition_point(|e| e.0 < h);
        idx[start..]
            .iter()
            .take_while(|e| e.0 == h)
            .find(|e| self.path(PathId(e.1)).as_deref() == Some(path))
            .map(|e| PathId(e.1))
    }

    fn paths(&self) -> usize {
        self.counts[0] as usize
    }

    fn ident(&self, id: IdentId) -> Option<(Arc<str>, Arc<str>)> {
        self.idents.get(id.0 as usize).cloned()
    }

    fn idents(&self) -> usize {
        self.idents.len()
    }

    fn label(&self, id: LabelId) -> Option<Arc<str>> {
        self.labels.get(id.0 as usize).cloned()
    }

    fn find_label(&self, name: &str) -> Option<LabelId> {
        self.labels
            .iter()
            .position(|l| &**l == name)
            .and_then(|i| u32::try_from(i).ok())
            .map(LabelId)
    }
}
