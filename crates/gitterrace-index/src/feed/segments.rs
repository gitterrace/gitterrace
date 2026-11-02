//! [`InsightsSource`] over the segment files. Aggregates, postings
//! and rename edges come from the segments. Commit columns, file rows and strings come from
//! `base`, the record stream of step 1, whose ids must be the segments' ids (one numbering per
//! cache). A reader never compares ordinals of two segments: each segment's local
//! ordinals become ranks through its own rank section, read by pread, never held in memory.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, Stream, Sums,
};
use crate::seg::agg::{self, Aggregates, Strings};
use crate::seg::bounds::Cursor;
use crate::seg::format::section;
use crate::seg::postings::{self, Postings};
use crate::seg::read::Container;
use crate::seg::spill::Spill;
use crate::seg::{Ordinal, SegResult, Sink};

/// Writes the tables of one segment in section order: postings (3, 4, 5), aggregates (6, 8)
/// and the rank of each local ordinal (33).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the merge (step 2) and tests write tables whole")
)]
pub(crate) fn write_tables(
    agg: &Aggregates,
    post: &Postings,
    ranks: &[u32],
    sink: &mut Sink<'_>,
) -> SegResult<()> {
    post.encode(sink)?;
    agg.encode(sink)?;
    pack_ranks(ranks, sink)
}

/// Builds the tables of one segment from records in canonical order (the store's writer feeds
/// it). Path postings go through the spill, so a linux build holds at most one run of them.
pub(crate) struct SegmentBuilder {
    agg: Aggregates,
    post: Postings,
    spill: Spill,
    ranks: Vec<u32>,
}

impl SegmentBuilder {
    /// `open` creates the spill's run file (a temp file in the cache directory).
    pub(crate) fn new(open: impl FnMut() -> std::io::Result<std::fs::File> + 'static) -> Self {
        Self::with_spill(Spill::for_build(open))
    }

    fn with_spill(spill: Spill) -> Self {
        Self {
            agg: Aggregates::default(),
            post: Postings::default(),
            spill,
            ranks: Vec::new(),
        }
    }

    /// Folds the next record. `strings` is the cache's interner (ids global until a merge), `rank`
    /// the record's position in the order file.
    pub(crate) fn push(
        &mut self,
        strings: &mut Strings,
        r: &crate::CommitRecord,
        rank: u32,
    ) -> SegResult<()> {
        let (who, paths) = strings.ids(r);
        let ord = u32::try_from(self.ranks.len()).unwrap_or(u32::MAX);
        self.ranks.push(rank);
        self.agg.push(r, who, &paths);
        // Path hits go to the spill, the rest stays in the (small) in-memory tables.
        self.post.push_without_paths(strings, r, who, &paths);
        if !r.merge {
            for (row, &p) in paths.iter().enumerate() {
                self.spill
                    .push((p, ord, u32::try_from(row).unwrap_or(u32::MAX)))?;
            }
        }
        Ok(())
    }

    /// Writes sections 3, 4, 5, 6, 8 and 33 in id order.
    pub(crate) fn finish(self, sink: &mut Sink<'_>) -> SegResult<()> {
        postings::encode_spilled(self.spill, sink)?;
        self.post.encode_rest(sink)?;
        self.agg.encode(sink)?;
        pack_ranks(&self.ranks, sink)
    }
}

fn pack_ranks(ranks: &[u32], sink: &mut Sink<'_>) -> SegResult<()> {
    agg::pack(
        section::RANK,
        ranks.iter().copied(),
        |_, r, out| out.extend_from_slice(&r.to_le_bytes()),
        sink,
    )
}

/// Segments plus the source of commits and strings.
pub struct SegmentSource<B> {
    base: B,
    segments: Vec<Container>,
    /// The first store error a stream met, so the index can rebuild (design 6).
    broken: std::sync::Mutex<Option<String>>,
}

impl<B: InsightsSource> SegmentSource<B> {
    pub const fn new(base: B, segments: Vec<Container>) -> Self {
        Self {
            base,
            segments,
            broken: std::sync::Mutex::new(None),
        }
    }

    /// The base, for its own error state.
    pub const fn base(&self) -> &B {
        &self.base
    }

    /// The first store error a stream of the segment tables met, if any.
    pub fn unreadable(&self) -> Option<String> {
        self.broken.lock().ok().and_then(|b| b.clone())
    }

    /// A store error inside a stream: logged with its file and offset (design 6), remembered
    /// for [`SegmentSource::unreadable`], then the stream is empty.
    #[expect(
        clippy::print_stderr,
        reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
    )]
    fn or_log<T>(&self, r: SegResult<Vec<T>>) -> Vec<T> {
        r.unwrap_or_else(|e| {
            eprintln!("gitterrace: {e}");
            if let Ok(mut b) = self.broken.lock() {
                b.get_or_insert_with(|| e.to_string());
            }
            Vec::new()
        })
    }

    /// Folds every block of `sec` in every segment through `f`. A store error ends the
    /// stream early: the caller of the source maps `Unreadable` to a rebuild (design 6).
    fn each_block(
        &self,
        sec: u16,
        mut f: impl FnMut(&Container, &mut Cursor<'_>, u32) -> SegResult<()>,
    ) -> SegResult<()> {
        let mut buf = Vec::new();
        for seg in &self.segments {
            for e in seg.blocks(sec) {
                let block = seg.read_block(e, &mut buf)?;
                let mut c = Cursor::new(block, seg.name(), e.offset);
                f(seg, &mut c, e.items)?;
            }
        }
        Ok(())
    }

    fn rank(seg: &Container, ordinal: u32) -> SegResult<Rank> {
        seg.rank(Ordinal(ordinal))
    }

    fn author_day(&self, days: &Range<i64>) -> SegResult<Vec<AuthorDayRow>> {
        let mut out: BTreeMap<(i64, u32), AuthorDayRow> = BTreeMap::new();
        self.each_block(section::AUTHOR_DAY, |_, c, n| {
            agg::decode_author_day(c, n, |day, who, v| {
                if !days.contains(&day) {
                    return;
                }
                let e = out.entry((day, who)).or_insert_with(|| AuthorDayRow {
                    day,
                    ident: IdentId(who),
                    commits: 0,
                    sums: Sums::default(),
                });
                e.commits += v.commits;
                e.sums.merge(&sums(&v.lines));
            })
        })?;
        Ok(out.into_values().collect())
    }

    fn path_author(&self, prefix: &str) -> SegResult<Vec<PathAuthorRow>> {
        let mut keep: BTreeMap<u32, bool> = BTreeMap::new();
        let mut out: BTreeMap<(u32, u32), PathAuthorRow> = BTreeMap::new();
        self.each_block(section::PATH_AUTHOR, |_, c, n| {
            agg::decode_path_author(c, n, |path, who, v| {
                let ok = *keep.entry(path).or_insert_with(|| {
                    self.base
                        .path(PathId(path))
                        .is_some_and(|p| prefix.is_empty() || crate::insights::under(&p, prefix))
                });
                if !ok {
                    return;
                }
                let e = out.entry((path, who)).or_insert_with(|| PathAuthorRow {
                    path: PathId(path),
                    ident: IdentId(who),
                    touches: 0,
                    sums: Sums::default(),
                });
                e.touches += v.touches;
                e.sums.merge(&sums(&v.lines));
            })
        })?;
        Ok(out.into_values().collect())
    }

    fn hits(&self, key: Posting) -> SegResult<Vec<Hit>> {
        let mut out = Vec::new();
        match key {
            Posting::Path(want) => {
                let mut local = Vec::new();
                self.each_block(section::PATH_POSTINGS, |seg, c, n| {
                    local.clear();
                    postings::decode_path_postings(c, n, |id, hits| {
                        if id == want.0 {
                            local.extend_from_slice(hits);
                        }
                    })?;
                    for h in &local {
                        out.push(Hit {
                            rank: Self::rank(seg, h.ordinal)?,
                            row: h.row,
                        });
                    }
                    Ok(())
                })?;
            }
            Posting::Author(want) => {
                let mut local = Vec::new();
                self.each_block(section::AUTHOR_POSTINGS, |seg, c, n| {
                    local.clear();
                    postings::decode_author_postings(c, n, |id, ords| {
                        if id == want.0 {
                            local.extend_from_slice(ords);
                        }
                    })?;
                    for &o in &local {
                        out.push(Hit {
                            rank: Self::rank(seg, o)?,
                            row: 0,
                        });
                    }
                    Ok(())
                })?;
            }
        }
        // Cross-segment order is the canonical key, which rank alone decides.
        out.sort_unstable();
        Ok(out)
    }

    fn edges(&self) -> SegResult<Vec<RenameEdge>> {
        let mut local = Vec::new();
        let mut out = Vec::new();
        self.each_block(section::RENAMES, |seg, c, n| {
            local.clear();
            postings::decode_renames(c, n, |e| local.push(e))?;
            for e in &local {
                out.push(RenameEdge {
                    rank: Self::rank(seg, e.ordinal)?,
                    row: e.row,
                    old: PathId(e.old),
                    new: PathId(e.new),
                });
            }
            Ok(())
        })?;
        out.sort_unstable();
        Ok(out)
    }
}

const fn sums(s: &agg::Sums) -> Sums {
    Sums {
        add: s.add,
        del: s.del,
        rewritten: s.rewritten,
        code_add: s.code_add,
        code_del: s.code_del,
        code_rewritten: s.code_rewritten,
    }
}

impl<B: InsightsSource> InsightsSource for SegmentSource<B> {
    fn len(&self) -> usize {
        self.base.len()
    }

    fn commit(&self, rank: Rank) -> Option<CommitRow> {
        self.base.commit(rank)
    }

    fn commits_in(&self, range: CommitRange) -> Stream<'_, CommitRow> {
        self.base.commits_in(range)
    }

    fn rows_for(&self, commits: Vec<Rank>) -> Stream<'_, (Rank, Vec<FileRow>)> {
        self.base.rows_for(commits)
    }

    fn commit_id(&self, rank: Rank) -> Option<Arc<str>> {
        self.base.commit_id(rank)
    }

    fn parent_ids(&self, rank: Rank) -> Vec<Arc<str>> {
        self.base.parent_ids(rank)
    }

    fn find_commit(&self, id: &str) -> Option<Rank> {
        self.base.find_commit(id)
    }

    fn agg_author_day(&self, days: Range<i64>) -> Stream<'_, AuthorDayRow> {
        Box::new(self.or_log(self.author_day(&days)).into_iter())
    }

    fn agg_path_author(&self, prefix: &str) -> Stream<'_, PathAuthorRow> {
        Box::new(self.or_log(self.path_author(prefix)).into_iter())
    }

    fn postings(&self, key: Posting) -> Stream<'_, Hit> {
        Box::new(self.or_log(self.hits(key)).into_iter())
    }

    fn rename_edges(&self) -> Stream<'_, RenameEdge> {
        Box::new(self.or_log(self.edges()).into_iter())
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

#[cfg(test)]
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the segment parts of a fixture"
)]
mod tests {
    use std::path::Path;

    use super::{SegmentSource, write_tables};
    use crate::CommitRecord;
    use crate::feed::{InsightsSource, MemorySource, Posting};
    use crate::insights::fixtures::{file, rec};
    use crate::seg::SegResult;
    use crate::seg::agg::{Aggregates, Strings};
    use crate::seg::format::{Kind, StringBase};
    use crate::seg::postings::Postings;
    use crate::seg::read::Container;
    use crate::seg::write::ContainerWriter;

    fn renamed(new: &str, old: &str) -> crate::FileStat {
        let mut f = file(new, 1, 1, 5);
        f.old = Some(old.into());
        f
    }

    fn history() -> Vec<CommitRecord> {
        let mut m = rec("m", "Bob", 6, vec![file("a", 1, 0, 1)]);
        m.merge = true;
        vec![
            rec("1", "Ann", 0, vec![file("a", 1, 0, 1), file("b", 1, 0, 1)]),
            rec("2", "Ann", 1, vec![renamed("c", "a")]),
            rec("3", "Bob", 2, vec![file("a", 2, 0, 2), file("c", 1, 0, 2)]),
            rec("4", "Bob", 3, vec![renamed("a", "b"), renamed("b", "a")]),
            rec("5", "Ann", 40, vec![renamed("d", "c"), file("a", 1, 1, 2)]),
            m,
            rec("6", "Cy", 41, vec![renamed("e", "d")]),
        ]
    }

    /// One segment per slice of `recs`, `ranks` giving each commit's canonical position.
    fn write(
        dir: &Path,
        s: &mut Strings,
        n: usize,
        recs: &[&CommitRecord],
        ranks: &[u32],
    ) -> SegResult<Container> {
        let (mut a, mut p) = (Aggregates::default(), Postings::default());
        for r in recs {
            let (who, paths) = s.ids(r);
            a.push(r, who, &paths);
            p.push(s, r, who, &paths);
        }
        let name = format!("seg-1-{n}.gts");
        let mut w = ContainerWriter::new(dir, &name, Kind::Segment, StringBase::default())?;
        write_tables(&a, &p, ranks, &mut |sec, items, b| w.block(sec, items, b))?;
        w.finish()?;
        Container::open(dir, &name, Kind::Segment)
    }

    /// Every aggregate, posting and edge from one segment and from an unmerged two-segment
    /// set (a branch written later, dated earlier) equals the memory adapter.
    #[test]
    fn segments_answer_like_the_memory_adapter() -> SegResult<()> {
        let recs = history();
        let mem = MemorySource::new(&recs);
        let all: Vec<&CommitRecord> = recs.iter().collect();
        let dir = tempfile::tempdir()?;
        let mut s = Strings::default();
        let one = vec![write(dir.path(), &mut s, 0, &all, &[0, 1, 2, 3, 4, 5, 6])?];
        // Two-step: commits 1 2 4 5 m 6 first, then the branch commit 3. Path ids still
        // follow the memory adapter's first use, as the cache's string deltas would.
        let dir2 = tempfile::tempdir()?;
        let mut s2 = Strings::default();
        for r in &recs {
            s2.ids(r);
        }
        let first: Vec<&CommitRecord> = [0, 1, 3, 4, 5, 6].map(|i| &recs[i]).to_vec();
        let a = write(dir2.path(), &mut s2, 0, &first, &[0, 1, 3, 4, 5, 6])?;
        let b = write(dir2.path(), &mut s2, 1, &[&recs[2]], &[2])?;
        for segs in [one, vec![a, b]] {
            let src = SegmentSource::new(MemorySource::new(&recs), segs);
            let n = segs_len(&src);
            assert_eq!(
                src.agg_author_day(i64::MIN..i64::MAX).collect::<Vec<_>>(),
                mem.agg_author_day(i64::MIN..i64::MAX).collect::<Vec<_>>(),
                "{n} segments"
            );
            for prefix in ["", "a", "zz"] {
                assert_eq!(
                    src.agg_path_author(prefix).collect::<Vec<_>>(),
                    mem.agg_path_author(prefix).collect::<Vec<_>>(),
                );
            }
            for p in 0..mem.paths() {
                let k = Posting::Path(crate::feed::PathId(u32::try_from(p).unwrap_or(0)));
                assert_eq!(
                    src.postings(k).collect::<Vec<_>>(),
                    mem.postings(k).collect::<Vec<_>>()
                );
            }
            for i in 0..mem.idents() {
                let k = Posting::Author(crate::feed::IdentId(u32::try_from(i).unwrap_or(0)));
                assert_eq!(
                    src.postings(k).collect::<Vec<_>>(),
                    mem.postings(k).collect::<Vec<_>>()
                );
            }
            assert_eq!(
                src.rename_edges().collect::<Vec<_>>(),
                mem.rename_edges().collect::<Vec<_>>()
            );
        }
        Ok(())
    }

    /// The builder (spilled path postings) writes the same bytes as the in-memory tables,
    /// whether runs were cut or not.
    #[test]
    fn builder_bytes_equal_table_bytes() {
        let recs = history();
        let ranks: Vec<u32> = (0..7).collect();
        let mut want: Vec<(u16, u32, Vec<u8>)> = Vec::new();
        let (mut a, mut p) = (Aggregates::default(), Postings::default());
        let mut s = Strings::default();
        for r in &recs {
            let (who, paths) = s.ids(r);
            a.push(r, who, &paths);
            p.push(&s, r, who, &paths);
        }
        assert!(
            write_tables(&a, &p, &ranks, &mut |x, n, b| {
                want.push((x, n, b.to_vec()));
                Ok(())
            })
            .is_ok()
        );
        for limit in [1, 2, usize::MAX] {
            let mut b = super::SegmentBuilder::with_spill(crate::seg::spill::Spill::new(
                limit,
                tempfile::tempfile,
            ));
            let mut s = Strings::default();
            for (r, &k) in recs.iter().zip(&ranks) {
                assert!(b.push(&mut s, r, k).is_ok());
            }
            let mut got: Vec<(u16, u32, Vec<u8>)> = Vec::new();
            assert!(
                b.finish(&mut |x, n, bytes| {
                    got.push((x, n, bytes.to_vec()));
                    Ok(())
                })
                .is_ok()
            );
            assert_eq!(got, want, "limit {limit}");
        }
        let _ = super::SegmentBuilder::new(tempfile::tempfile);
    }

    fn segs_len<B>(s: &SegmentSource<B>) -> usize {
        s.segments.len()
    }
}
