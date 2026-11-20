//! Segment tables at open, the record decoder and the small id helpers.

use crate::feed::Rank;
use crate::feed::disk::{DiskSource, Rec, Seg, columns};
use crate::feed::{FileRow, PathId, Sums};
use crate::seg::bounds::Cursor;
use crate::seg::format::{TocEntry, section};
use crate::seg::read::Container;
use crate::seg::{Oid, SegError, SegResult, hex};
use crate::{CodeStat, CommitRecord, FileStat};

pub(super) fn open_seg(c: Container, first_expected: u32, buf: &mut Vec<u8>) -> SegResult<Seg> {
    let total = |sec| c.blocks(sec).iter().map(|e| u64::from(e.items)).sum::<u64>();
    let n = total(section::RANK);
    let bad = |what| SegError::unreadable(c.name(), 0, what);
    if total(section::ROWS) != n
        || total(section::ROW_INDEX) != n
        || total(section::OID_INDEX) != n
        || total(section::COMMITS) != n
    {
        return Err(bad("segment rows missing or miscounted"));
    }
    let n = u32::try_from(n).map_err(|_| bad("too many rows"))?;
    // Ranks must be first..first+n, the reader maps rank to ordinal by subtraction.
    let mut want = first_expected;
    for e in c.blocks(section::RANK) {
        let b = c.read_block(e, buf)?;
        if b.len() != e.items as usize * 4 {
            return Err(bad("rank block length"));
        }
        for r in b.as_chunks::<4>().0 {
            if u32::from_le_bytes(*r) != want {
                return Err(bad("segment ranks not one contiguous run"));
            }
            want += 1;
        }
    }
    let mut row_starts = vec![0u64];
    for e in c.blocks(section::ROWS) {
        let last = *row_starts.last().unwrap_or(&0);
        row_starts.push(last + u64::from(e.len));
    }
    let fixed = |e: &TocEntry, w: u32| u64::from(e.items) * u64::from(w) == u64::from(e.len);
    let mut idx_first = Vec::new();
    let mut at = 0u32;
    for e in c.blocks(section::ROW_INDEX) {
        if !fixed(e, 8) {
            return Err(bad("row index block length"));
        }
        idx_first.push(at);
        at += e.items;
    }
    let mut oid_first = Vec::new();
    for e in c.blocks(section::OID_INDEX) {
        if !fixed(e, 24) || e.items == 0 {
            return Err(bad("oid index block length"));
        }
        // Routing only, read without the crc so open stays one small pread per block: a
        // lookup compares it with the checked block it routes to and with the next block's
        // checked first oid before it answers "absent" (`DiskSource::rank_of`).
        let mut oid = [0u8; 20];
        c.head_unchecked(e, &mut oid)?;
        oid_first.push(oid);
    }
    if !oid_first.is_sorted() {
        return Err(bad("oid index not sorted"));
    }
    let mut col_first = Vec::new();
    let mut at = 0u32;
    for e in c.blocks(section::COMMITS) {
        if !fixed(e, columns::ITEM as u32) {
            return Err(bad("commit column block length"));
        }
        col_first.push(at);
        at += e.items;
    }
    Ok(Seg { c, first: first_expected, n, row_starts, idx_first, oid_first, col_first })
}

/// Parses one record. File rows are pushed one at a time, never sized by the count: a
/// hostile count with a valid crc must fail at the end of the bytes, not allocate for it.
pub(super) fn parse(c: &mut Cursor<'_>, mut files: Option<&mut Vec<FileRow>>) -> SegResult<Rec> {
    let oid = c.array::<20>()?;
    let ident = c.u32()?;
    let time = i64::from_le_bytes(c.array()?);
    let offset = i32::from_le_bytes(c.array()?);
    let flags = c.u8()?;
    if flags > 1 {
        return Err(c.err("record flags"));
    }
    let opt = |v: u32| v.checked_sub(1);
    let kind = opt(c.u32()?);
    let scope = opt(c.u32()?);
    let np = c.u16()?;
    let mut parents = Vec::new();
    for _ in 0..np {
        parents.push(c.array::<20>()?);
    }
    let nf = c.u32()?;
    let mut sums = Sums::default();
    for _ in 0..nf {
        let path = PathId(c.u32()?);
        let old = opt(c.u32()?).map(PathId);
        let (add, del, lines, rewritten) = (c.u32()?, c.u32()?, c.u32()?, c.u32()?);
        let ff = c.u8()?;
        if ff > 7 {
            return Err(c.err("file flags"));
        }
        let code = if ff & 2 != 0 { Some((c.u32()?, c.u32()?, c.u32()?)) } else { None };
        let f = FileRow {
            path,
            old,
            add,
            del,
            lines,
            rewritten,
            code,
            binary: ff & 1 != 0,
            deleted: ff & 4 != 0,
        };
        sums.add_row(&f).ok_or_else(|| c.err(crate::seg::agg::SUM_OVER))?;
        if let Some(out) = files.as_deref_mut() {
            out.push(f);
        }
    }
    Ok(Rec {
        oid,
        ident,
        time,
        offset,
        merge: flags & 1 != 0,
        kind,
        scope,
        parents,
        files: nf,
        sums,
    })
}

pub(super) fn oid_of(id: &str) -> SegResult<Oid> {
    crate::store::oid_of(id).ok_or_else(|| SegError::BadName(format!("commit id {id:?}")))
}

pub(super) fn hash32(s: &str) -> u32 {
    gitterrace_git::fnv1a32(s.bytes())
}

#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache error goes to the user's stderr"
)]
pub(super) fn report(e: &SegError) {
    eprintln!("gitterrace: {e}");
}

impl DiskSource {
    /// One record as stored, raw identity and raw time.
    pub(super) fn record(&self, rank: Rank) -> SegResult<CommitRecord> {
        let mut files = Vec::new();
        let rec = self.decode(rank, Some(&mut files))?;
        let who = self.raw_ident(rec.ident)?;
        let label =
            |l: Option<u32>| l.map(|i| self.string(2, i).map(|s| s.to_string())).transpose();
        let path = |p: u32| self.string(0, p);
        let mut stats = Vec::new();
        for f in files {
            stats.push(FileStat {
                path: path(f.path.0)?,
                additions: f.add,
                deletions: f.del,
                lines: f.lines,
                rewritten: f.rewritten,
                code: f.code.map(|(a, d, r)| CodeStat { additions: a, deletions: d, rewritten: r }),
                old: f.old.map(|o| path(o.0)).transpose()?,
                binary: f.binary,
                deleted: f.deleted,
            });
        }
        Ok(CommitRecord {
            id: hex(&rec.oid),
            author: who.0,
            email: who.1,
            time: rec.time,
            offset: rec.offset,
            merge: rec.merge,
            parents: rec.parents.iter().map(hex).collect(),
            kind: label(rec.kind)?,
            scope: label(rec.scope)?,
            files: stats,
        })
    }
}
