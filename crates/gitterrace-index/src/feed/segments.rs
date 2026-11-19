//! [`InsightsSource`] over the segment files. Aggregates, postings
//! and rename edges come from the segments. Commit columns, file rows and strings come from
//! `base`, the record stream, whose ids must be the segments' ids (one numbering per
//! cache). A reader never compares ordinals of two segments: each segment's local
//! ordinals become ranks through its own rank section, read by pread, never held in memory.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, SourceError, Stream, Sums,
};
use crate::seg::agg::{self, Aggregates, Strings};
use crate::seg::bounds::Cursor;
use crate::seg::format::section;
use crate::seg::postings::{self, Postings};
use crate::seg::read::Container;
use crate::seg::spill::Spill;
use crate::seg::{SegResult, Sink};

/// Writes the tables of one segment in section order: postings (3, 4, 5), aggregates (6 to 10)
/// and the rank of each local ordinal (33).
// Tests write tables whole.
#[cfg(test)]
pub(crate) fn write_tables(
    agg: &Aggregates,
    post: &Postings,
    ranks: &[u32],
    paths: usize,
    sink: &mut Sink<'_>,
) -> SegResult<()> {
    post.encode(sink)?;
    agg.encode(paths, sink)?;
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
        let olds = crate::seg::canon::olds(strings, r);
        let ord = u32::try_from(self.ranks.len()).unwrap_or(u32::MAX);
        self.ranks.push(rank);
        self.agg.push(r, who, &paths, &olds);
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

    /// Continues the life numbering of the set this segment is appended to.
    pub(crate) fn continue_lives(&mut self, lives: crate::seg::canon::Lives) {
        self.agg.lives = lives;
    }

    /// Writes sections 3 to 10 and 33 in id order. `paths` is the set's path count after
    /// this segment.
    /// Returns the life state after it, for a writer that goes on with another segment.
    pub(crate) fn finish(
        self,
        paths: usize,
        sink: &mut Sink<'_>,
    ) -> SegResult<crate::seg::canon::Lives> {
        postings::encode_spilled(self.spill, sink)?;
        self.post.encode_rest(sink)?;
        self.agg.encode(paths, sink)?;
        pack_ranks(&self.ranks, sink)?;
        Ok(self.agg.lives)
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
    /// Per segment, the first path id of each path postings block, read on first lookup: a
    /// path's hits then cost the blocks that hold it, not the whole section.
    path_index: std::sync::OnceLock<Vec<Vec<u32>>>,
    /// See [`InsightsSource::instance`].
    #[cfg(any(test, feature = "oracle"))]
    instance: u64,
}

impl<B: InsightsSource> SegmentSource<B> {
    #[cfg_attr(
        not(any(test, feature = "oracle")),
        expect(
            clippy::missing_const_for_fn,
            reason = "const only without the instance id, which reads an atomic"
        )
    )]
    pub fn new(base: B, segments: Vec<Container>) -> Self {
        Self {
            base,
            segments,
            broken: std::sync::Mutex::new(None),
            path_index: std::sync::OnceLock::new(),
            #[cfg(any(test, feature = "oracle"))]
            instance: crate::feed::next_instance(),
        }
    }

    /// First path id per path postings block, per segment. A block starts with an absolute
    /// id (a delta from 0), so this reads one varint per block.
    fn path_index(&self) -> SegResult<&[Vec<u32>]> {
        if let Some(i) = self.path_index.get() {
            return Ok(i);
        }
        let mut buf = Vec::new();
        let mut all = Vec::with_capacity(self.segments.len());
        for seg in &self.segments {
            let mut firsts = Vec::new();
            for e in seg.blocks(section::PATH_POSTINGS) {
                let block = seg.read_block(e, &mut buf)?;
                let mut c = Cursor::new(block, seg.name(), e.offset);
                let first = c.varint()?;
                firsts.push(u32::try_from(first).map_err(|_| c.err("id over u32"))?);
            }
            all.push(firsts);
        }
        Ok(self.path_index.get_or_init(|| all))
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

    fn author_day(&self, days: &Range<i64>) -> SegResult<Vec<AuthorDayRow>> {
        let mut out: BTreeMap<(i64, u32), AuthorDayRow> = BTreeMap::new();
        self.each_block(section::AUTHOR_DAY, |_, c, n| {
            agg::decode_author_day(c, n, |day, who, v| {
                // A merge-only row: the commit counts leave merges out until they are opted in.
                if !days.contains(&day) || v.commits == 0 {
                    return Some(());
                }
                let e = out.entry((day, who)).or_insert_with(|| AuthorDayRow {
                    day,
                    ident: IdentId(who),
                    commits: 0,
                    sums: Sums::default(),
                });
                agg::add_count(&mut e.commits, v.commits)?;
                e.sums.merge(&sums(&v.lines))
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
                    return Some(());
                }
                let e = out.entry((path, who)).or_insert_with(|| PathAuthorRow {
                    path: PathId(path),
                    ident: IdentId(who),
                    touches: 0,
                    sums: Sums::default(),
                });
                agg::add_count(&mut e.touches, v.touches)?;
                e.sums.merge(&sums(&v.lines))
            })
        })?;
        Ok(out.into_values().collect())
    }

    fn hits(&self, key: Posting) -> SegResult<Vec<Hit>> {
        let mut out = Vec::new();
        match key {
            Posting::Path(want) => {
                let index = self.path_index()?;
                let mut local = Vec::new();
                let mut buf = Vec::new();
                for (seg, firsts) in self.segments.iter().zip(index) {
                    let blocks = seg.blocks(section::PATH_POSTINGS);
                    // Blocks whose first id is at most `want`, from the last one that starts
                    // below it (a long list continues across blocks with the same id).
                    let end = firsts.partition_point(|&f| f <= want.0);
                    let start = firsts[..end].iter().rposition(|&f| f < want.0).unwrap_or(0);
                    for e in &blocks[start..end] {
                        let block = seg.read_block(e, &mut buf)?;
                        let mut c = Cursor::new(block, seg.name(), e.offset);
                        local.clear();
                        postings::decode_path_postings(&mut c, e.items, |id, hits| {
                            if id == want.0 {
                                local.extend_from_slice(hits);
                            }
                        })?;
                        let ords: Vec<u32> = local.iter().map(|h| h.ordinal).collect();
                        for (h, rank) in local.iter().zip(seg.ranks(&ords)?) {
                            out.push(Hit { rank, row: h.row });
                        }
                    }
                }
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
                    for rank in seg.ranks(&local)? {
                        out.push(Hit { rank, row: 0 });
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
            let ords: Vec<u32> = local.iter().map(|e| e.ordinal).collect();
            for (e, rank) in local.iter().zip(seg.ranks(&ords)?) {
                out.push(RenameEdge {
                    rank,
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

    fn find_commit(&self, id: &str) -> Result<Option<Rank>, SourceError> {
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

    #[cfg(any(test, feature = "oracle"))]
    fn instance(&self) -> u64 {
        self.instance
    }

    fn failure(&self) -> Option<SourceError> {
        self.unreadable()
            .map(SourceError)
            .or_else(|| self.base.failure())
    }

    fn tip_touches(&self, each: &mut dyn FnMut(Rank, &[crate::feed::TipTouch])) -> bool {
        self.base.tip_touches(each)
    }

    fn has_tip(&self) -> bool {
        self.base.has_tip()
    }
}

#[cfg(test)]
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the segment parts of a fixture"
)]
mod tests;
