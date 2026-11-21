//! The `InsightsSource` trait over the disk base.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::feed::disk::record::hash32;
use crate::feed::disk::{DiskSource, scan};
use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, SourceError, Stream, fold_author_day,
    fold_path_author,
};
use crate::seg::hex;

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
        // Ascending ranks: one forward scan that seeks to each, so every block is read once
        // per call, whatever the spread. A lookup per commit goes through the shared block
        // cache, and a pass over a large history evicts the blocks it needs next: on a history
        // whose index blocks do not fit the cache, each commit re-read and re-checked several.
        if commits.len() > 1 && commits.windows(2).all(|w| w[0] < w[1]) {
            let to = commits[commits.len() - 1].0.saturating_add(1);
            let mut s = scan::Scan::new(self, commits[0].0, to, true);
            let mut want = commits.into_iter();
            return Box::new(std::iter::from_fn(move || {
                let next = want.next()?;
                s.seek(next.0);
                let mut files = Vec::new();
                let c = s.next_row(&mut files)?;
                Some((c.rank, files))
            }));
        }
        Box::new(commits.into_iter().filter_map(move |r| {
            if r.0 >= self.len {
                return None;
            }
            let mut files = Vec::new();
            self.ok(self.row_with(r, Some(&mut files))).map(|_| (r, files))
        }))
    }

    fn commit_id(&self, rank: Rank) -> Option<Arc<str>> {
        if rank.0 >= self.len {
            return None;
        }
        self.ok(self.decode(rank, None)).map(|r| Arc::from(hex(&r.oid)))
    }

    fn parent_ids(&self, rank: Rank) -> Vec<Arc<str>> {
        if rank.0 >= self.len {
            return Vec::new();
        }
        self.ok(self.decode(rank, None))
            .map(|r| r.parents.iter().map(|p| Arc::from(hex(p))).collect())
            .unwrap_or_default()
    }

    fn find_commit(&self, id: &str) -> Result<Option<Rank>, SourceError> {
        let Some(oid) = crate::store::oid_of(id) else {
            return Ok(None);
        };
        self.checked(self.rank_of(&oid))
    }

    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow> {
        fold_author_day(self.commits_in(CommitRange::All), &days, &self.over)
    }

    fn agg_path_author(&self, prefix: &str) -> Stream<'_, PathAuthorRow> {
        let prefix = prefix.trim_matches('/');
        let mut keep: BTreeMap<PathId, bool> = BTreeMap::new();
        fold_path_author(
            &self.over,
            |p| {
                *keep.entry(p).or_insert_with(|| {
                    prefix.is_empty()
                        || self.path(p).is_some_and(|n| crate::insights::under(&n, prefix))
                })
            },
            |f| self.each_row(f),
        )
    }

    fn postings(&self, key: Posting) -> Stream<'_, Hit> {
        let mut out = Vec::new();
        self.each_row(|c, files| match key {
            Posting::Path(p) if !c.merge => {
                for (i, f) in files.iter().enumerate() {
                    if f.path == p {
                        out.push(Hit { rank: c.rank, row: u32::try_from(i).unwrap_or(u32::MAX) });
                    }
                }
            }
            Posting::Author(a) if c.ident == a => out.push(Hit { rank: c.rank, row: 0 }),
            _ => {}
        });
        Box::new(out.into_iter())
    }

    fn rename_edges(&self) -> Stream<'_, RenameEdge> {
        let mut out = Vec::new();
        self.each_row(|c, files| {
            out.extend(RenameEdge::of_rows(c.rank, files));
        });
        Box::new(out.into_iter())
    }

    fn path(&self, id: PathId) -> Option<Arc<str>> {
        if id.0 >= self.counts[0] {
            return None;
        }
        self.ok(self.string(0, id.0))
    }

    fn find_path(&self, path: &str) -> Result<Option<PathId>, SourceError> {
        let idx = if let Some(i) = self.path_hash.get() {
            i
        } else {
            let built = self.checked(self.path_index())?;
            self.path_hash.get_or_init(|| built)
        };
        let h = hash32(path);
        let start = idx.partition_point(|e| e.0 < h);
        for e in idx[start..].iter().take_while(|e| e.0 == h) {
            if *self.checked(self.string(0, e.1))? == *path {
                return Ok(Some(PathId(e.1)));
            }
        }
        Ok(None)
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

    fn find_label(&self, name: &str) -> Result<Option<LabelId>, SourceError> {
        Ok(self
            .labels
            .iter()
            .position(|l| &**l == name)
            .and_then(|i| u32::try_from(i).ok())
            .map(LabelId))
    }

    #[cfg(any(test, feature = "oracle"))]
    fn instance(&self) -> u64 {
        self.instance
    }

    fn failure(&self) -> Option<SourceError> {
        self.unreadable().map(SourceError).or_else(|| self.over.failure())
    }

    fn tip_touches(&self, each: &mut dyn FnMut(Rank, &[crate::feed::TipTouch])) -> bool {
        self.stored_tip(each)
    }

    fn has_tip(&self) -> bool {
        self.stored_has_tip()
    }
}
