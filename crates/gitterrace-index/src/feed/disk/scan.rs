//! Sequential reads of the commit records, in file order.
//!
//! A full pass (every page builds its tables from one) read each record through the block
//! cache: a hash probe, an index lookup and a block clone per commit, hundreds of thousands
//! on a large history. Here each `ROWS` and `ROW_INDEX` block is read once, in order, into
//! buffers owned by the scan. Memory is two blocks plus the record that spans a block edge.

use crate::feed::{CommitRow, FileRow, Rank};
use crate::seg::SegResult;
use crate::seg::bounds::Cursor;
use crate::seg::format::section;

use crate::feed::disk::columns::{self, ITEM};
use crate::feed::disk::{DiskSource, MAX_RECORD, Rec, parse};

/// Records of ranks `from..to`, ascending, with their file rows when `rows` is set.
pub(super) struct Scan<'s> {
    src: &'s DiskSource,
    rows: bool,
    rank: u32,
    to: u32,
    /// Segment, ordinal inside it.
    seg: usize,
    ord: u32,
    /// Current `ROWS` block (index among the segment's row blocks) and its bytes.
    row_block: usize,
    row_bytes: Vec<u8>,
    /// Current `ROW_INDEX` block and its bytes.
    idx_block: usize,
    idx_bytes: Vec<u8>,
    /// A record that spans blocks, gathered here.
    joined: Vec<u8>,
    /// Current `COMMITS` block and its bytes.
    col_block: usize,
    col_bytes: Vec<u8>,
    /// Block reads so far, for the guard.
    pub(super) reads: usize,
    done: bool,
}

impl<'s> Scan<'s> {
    pub(super) fn new(src: &'s DiskSource, from: u32, to: u32, rows: bool) -> Self {
        let to = to.min(src.len);
        let (seg, ord) = src.seg_of(Rank(from)).unwrap_or((src.segs.len(), 0));
        Self {
            src,
            rows,
            rank: from,
            to,
            seg,
            ord,
            row_block: usize::MAX,
            row_bytes: Vec::new(),
            idx_block: usize::MAX,
            idx_bytes: Vec::new(),
            joined: Vec::new(),
            col_block: usize::MAX,
            col_bytes: Vec::new(),
            reads: 0,
            done: from >= to,
        }
    }

    /// Byte offset of ordinal `o` of the current segment in its row payload.
    fn offset(&mut self, o: u32) -> SegResult<u64> {
        let seg = &self.src.segs[self.seg];
        let bi = seg.idx_first.partition_point(|&f| f <= o) - 1;
        if bi != self.idx_block {
            let e = seg.c.blocks(section::ROW_INDEX)[bi];
            seg.c.read_block(&e, &mut self.idx_bytes)?;
            self.reads += 1;
            self.idx_block = bi;
        }
        let at = (o - seg.idx_first[bi]) as usize * 8;
        let b = self
            .idx_bytes
            .get(at..at + 8)
            .and_then(|s| <[u8; 8]>::try_from(s).ok())
            .ok_or_else(|| crate::seg::SegError::unreadable(seg.c.name(), 0, "row index short"))?;
        Ok(u64::from_le_bytes(b))
    }

    /// The record of the current ordinal.
    fn record(&mut self, files: &mut Vec<FileRow>) -> SegResult<Rec> {
        let (s, o) = (self.seg, self.ord);
        let seg = &self.src.segs[s];
        let name = seg.c.name();
        let at = self.offset(o)?;
        let end = if o + 1 < seg.n {
            self.offset(o + 1)?
        } else {
            *seg.row_starts.last().unwrap_or(&0)
        };
        if end <= at || end - at > MAX_RECORD as u64 {
            return Err(crate::seg::SegError::unreadable(name, at, "row offsets"));
        }
        let seg = &self.src.segs[s];
        let blocks = seg.c.blocks(section::ROWS);
        let mut bi = seg.row_starts.partition_point(|&p| p <= at) - 1;
        self.joined.clear();
        let mut single = None;
        loop {
            if bi != self.row_block {
                let e = *blocks.get(bi).ok_or_else(|| {
                    crate::seg::SegError::unreadable(name, at, "record past the rows")
                })?;
                seg.c.read_block(&e, &mut self.row_bytes)?;
                self.reads += 1;
                self.row_block = bi;
            }
            let start = seg.row_starts[bi];
            let from = usize::try_from(at.saturating_sub(start)).unwrap_or(usize::MAX);
            let upto = usize::try_from((end - start).min(self.row_bytes.len() as u64)).unwrap_or(0);
            let part = self.row_bytes.get(from.min(upto)..upto).unwrap_or_default();
            if self.joined.is_empty() && end <= start + self.row_bytes.len() as u64 {
                single = Some((from, upto));
                break;
            }
            self.joined.extend_from_slice(part);
            if self.joined.len() as u64 >= end - at {
                break;
            }
            bi += 1;
        }
        let bytes: &[u8] = match single {
            Some((a, b)) => &self.row_bytes[a..b],
            None => &self.joined,
        };
        let mut c = Cursor::new(bytes, name, at);
        files.clear();
        let rec = parse(&mut c, self.rows.then_some(files))?;
        if !c.is_empty() {
            return Err(c.err("record shorter than its index span"));
        }
        Ok(rec)
    }

    /// The current ordinal's item of section 1.
    fn column(&mut self) -> SegResult<columns::Item> {
        let seg = &self.src.segs[self.seg];
        let o = self.ord;
        let bi = seg.col_first.partition_point(|&f| f <= o) - 1;
        if bi != self.col_block {
            let e = seg.c.blocks(section::COMMITS)[bi];
            seg.c.read_block(&e, &mut self.col_bytes)?;
            self.reads += 1;
            self.col_block = bi;
        }
        let at = (o - seg.col_first[bi]) as usize * ITEM;
        let b = self.col_bytes.get(at..at + ITEM).ok_or_else(|| {
            crate::seg::SegError::unreadable(seg.c.name(), 0, "commit column short")
        })?;
        columns::decode(&mut Cursor::new(b, seg.c.name(), at as u64))
    }

    /// The current commit: from section 1 alone, or with its record when rows are wanted,
    /// in which case the two must agree (a disagreement is a corrupt cache, never a guess).
    fn current(&mut self, files: &mut Vec<FileRow>) -> SegResult<columns::Item> {
        let it = self.column()?;
        if self.rows {
            let rec = self.record(files)?;
            if !it.agrees(&rec) {
                let name = self.src.segs[self.seg].c.name();
                return Err(crate::seg::SegError::unreadable(
                    name,
                    0,
                    "commit columns differ from rows",
                ));
            }
        }
        Ok(it)
    }

    /// The next commit and, when asked for, its file rows.
    pub(super) fn next_row(&mut self, files: &mut Vec<FileRow>) -> Option<CommitRow> {
        if self.done || self.rank >= self.to {
            return None;
        }
        let it = match self.current(files) {
            Ok(r) => r,
            Err(e) => {
                self.done = true;
                return self.src.fail(&e);
            }
        };
        let row = self.src.commit_row(Rank(self.rank), &it);
        self.rank += 1;
        self.ord += 1;
        if self.ord >= self.src.segs[self.seg].n {
            self.seg += 1;
            self.ord = 0;
            self.row_block = usize::MAX;
            self.idx_block = usize::MAX;
            self.col_block = usize::MAX;
        }
        Some(row)
    }
}

impl Iterator for Scan<'_> {
    type Item = CommitRow;

    fn next(&mut self) -> Option<CommitRow> {
        let mut none = Vec::new();
        self.next_row(&mut none)
    }
}

#[cfg(test)]
mod tests {
    use crate::feed::{CommitRange, InsightsSource, Rank};
    use crate::seg::SegResult;
    use crate::seg::format::section;
    use crate::seg::read::Store;
    use crate::{CommitRecord, FileStat};

    use super::super::DiskSource;
    use super::super::testing::write_cache;
    use super::Scan;

    /// Records big enough that rows span several blocks and some records cross block edges.
    fn records() -> Vec<CommitRecord> {
        (0..3000usize)
            .map(|i| CommitRecord {
                id: format!("{:040x}", i * 7919 + 1),
                author: ["Ann", "Bob"][i % 2].into(),
                email: "x@y".into(),
                time: 1_600_000_000 + i as i64 * 60,
                parents: (i > 0)
                    .then(|| format!("{:040x}", (i - 1) * 7919 + 1))
                    .into_iter()
                    .collect(),
                files: (0..(i % 120))
                    .map(|n| FileStat {
                        path: format!("dir{}/file{n}.rs", i % 13).into(),
                        additions: n as u32,
                        ..FileStat::default()
                    })
                    .collect(),
                ..CommitRecord::default()
            })
            .collect()
    }

    fn open(dir: &std::path::Path) -> SegResult<DiskSource> {
        let store =
            Store::open(dir)?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
        DiskSource::open(store, &mut |p| Ok(p.to_vec()))
    }

    /// A full scan reads every row and row index block exactly once and answers what the
    /// per-commit lookup answers, across segments and block edges. Red on the cache path,
    /// which reads a row index block per commit.
    #[test]
    fn a_full_scan_reads_each_block_once_and_equals_lookups() -> SegResult<()> {
        let recs = records();
        let dir = tempfile::tempdir()?;
        write_cache(dir.path(), &recs, &[200, 2200])?;
        let disk = open(dir.path())?;
        let blocks: usize = disk
            .segs
            .iter()
            .map(|s| {
                s.c.blocks(section::ROWS).len()
                    + s.c.blocks(section::ROW_INDEX).len()
                    + s.c.blocks(section::COMMITS).len()
            })
            .sum();
        assert!(
            disk.segs
                .iter()
                .any(|s| s.c.blocks(section::ROWS).len() > 1),
            "the fixture must span row blocks"
        );
        let mut scan = Scan::new(&disk, 0, disk.len, true);
        let mut files = Vec::new();
        let mut n = 0u32;
        while let Some(row) = scan.next_row(&mut files) {
            assert_eq!(Some(row), disk.commit(Rank(n)), "rank {n}");
            let want: Vec<_> = disk.rows_for(vec![Rank(n)]).flat_map(|(_, f)| f).collect();
            assert_eq!(files, want, "rows of {n}");
            n += 1;
        }
        assert_eq!(n as usize, recs.len());
        assert_eq!(scan.reads, blocks, "block reads per full scan");
        assert_eq!(disk.unreadable(), None);
        // The trait paths use the scan.
        assert_eq!(disk.commits_in(CommitRange::All).count(), recs.len());
        let all: Vec<Rank> = (0..disk.len).map(Rank).collect();
        assert_eq!(disk.rows_for(all).count(), recs.len());
        Ok(())
    }
}
