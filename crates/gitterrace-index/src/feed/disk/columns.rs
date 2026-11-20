//! Section 1, the commit columns: one fixed 80 B item per local ordinal.
//!
//! A pass over commits (every page builds its tables from one) reads these instead of the
//! records in `ROWS`, which carry every file row: about 28 MB instead of about 200 MB on a
//! history of rust's size. Time and offset are the raw values `ROWS` holds, the read clamps
//! time exactly as for a record. Sums are u64, so no sum is ever clamped.
//!
//! Item: author u32, time i64, offset i32, flags u8 (bit 0 merge), zero u8, zero u16,
//! kind + 1 u32 (0 none), scope + 1 u32, files u32 (32 B), then add, del, rewritten,
//! code add, code del, code rewritten as u64 (48 B).

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use crate::CommitRecord;
use crate::feed::Sums;
use crate::seg::bounds::{Cursor, MAX_BLOCK};
use crate::seg::format::section;
use crate::seg::{SegError, SegResult, Sink};

pub(super) const ITEM: usize = 80;
const PER_BLOCK: usize = MAX_BLOCK / ITEM;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Item {
    pub ident: u32,
    pub time: i64,
    pub offset: i32,
    pub merge: bool,
    pub kind: Option<u32>,
    pub scope: Option<u32>,
    pub files: u32,
    pub sums: Sums,
}

impl Item {
    /// Whether the record `ROWS` holds for the same ordinal says the same.
    pub(super) fn agrees(&self, r: &crate::feed::disk::Rec) -> bool {
        r.ident == self.ident
            && r.time == self.time
            && r.offset == self.offset
            && r.merge == self.merge
            && r.kind == self.kind
            && r.scope == self.scope
            && r.files == self.files
            && r.sums == self.sums
    }
}

/// The item of one record, ids as `ROWS` writes them.
pub(super) fn item(
    r: &CommitRecord,
    who: u32,
    kind: Option<u32>,
    scope: Option<u32>,
) -> [u8; ITEM] {
    let mut sums = Sums::default();
    for f in &r.files {
        let c = f.code_stat();
        sums.add += u64::from(f.additions);
        sums.del += u64::from(f.deletions);
        sums.rewritten += u64::from(f.rewritten);
        sums.code_add += u64::from(c.additions);
        sums.code_del += u64::from(c.deletions);
        sums.code_rewritten += u64::from(c.rewritten);
    }
    let opt = |v: Option<u32>| v.map_or(0, |v| v + 1);
    let mut out = [0u8; ITEM];
    let mut at = 0;
    let mut put = |b: &[u8]| {
        out[at..at + b.len()].copy_from_slice(b);
        at += b.len();
    };
    put(&who.to_le_bytes());
    put(&r.time.to_le_bytes());
    put(&r.offset.to_le_bytes());
    put(&[u8::from(r.merge), 0]);
    put(&0u16.to_le_bytes());
    put(&opt(kind).to_le_bytes());
    put(&opt(scope).to_le_bytes());
    put(&u32::try_from(r.files.len()).unwrap_or(u32::MAX).to_le_bytes());
    for v in [sums.add, sums.del, sums.rewritten, sums.code_add, sums.code_del, sums.code_rewritten]
    {
        put(&v.to_le_bytes());
    }
    out
}

/// Decodes one item. Every field is checked: a reserved byte that is not zero or a flag
/// outside the defined ones is `Unreadable`, never a guess.
pub(super) fn decode(c: &mut Cursor<'_>) -> SegResult<Item> {
    let ident = c.u32()?;
    let time = i64::from_le_bytes(c.array()?);
    let offset = i32::from_le_bytes(c.array()?);
    let flags = c.u8()?;
    if flags > 1 || c.u8()? != 0 || c.u16()? != 0 {
        return Err(c.err("commit column flags"));
    }
    let opt = |v: u32| v.checked_sub(1);
    let kind = opt(c.u32()?);
    let scope = opt(c.u32()?);
    let files = c.u32()?;
    let mut v = [0u64; 6];
    for x in &mut v {
        *x = u64::from_le_bytes(c.array()?);
    }
    Ok(Item {
        ident,
        time,
        offset,
        merge: flags & 1 != 0,
        kind,
        scope,
        files,
        sums: Sums {
            add: v[0],
            del: v[1],
            rewritten: v[2],
            code_add: v[3],
            code_del: v[4],
            code_rewritten: v[5],
        },
    })
}

/// Items spooled to a temp file during the build, so a long history costs one block of
/// memory, not one item per commit.
pub(super) struct Spool {
    path: PathBuf,
    file: BufWriter<File>,
    items: u64,
}

impl Spool {
    pub(super) fn new(dir: &Path, name: &str) -> SegResult<Self> {
        let file = crate::seg::io::create_no_follow(dir, name)?;
        Ok(Self { path: dir.join(name), file: BufWriter::new(file), items: 0 })
    }

    pub(super) fn push(&mut self, item: &[u8; ITEM]) -> SegResult<()> {
        self.file.write_all(item)?;
        self.items += 1;
        Ok(())
    }

    /// Writes the spooled items as section 1, whole items per block, and removes the spool.
    pub(super) fn finish(&mut self, sink: &mut Sink<'_>) -> SegResult<()> {
        self.file.flush()?;
        let mut f = self.file.get_ref().try_clone()?;
        std::io::Seek::rewind(&mut f)?;
        let mut left = self.items;
        let mut block = vec![0u8; PER_BLOCK * ITEM];
        while left > 0 {
            let n = usize::try_from(left.min(PER_BLOCK as u64)).unwrap_or(PER_BLOCK);
            f.read_exact(&mut block[..n * ITEM])?;
            let items =
                u32::try_from(n).map_err(|_| SegError::unreadable("columns", 0, "count"))?;
            sink(section::COMMITS, items, &block[..n * ITEM])?;
            left -= n as u64;
        }
        self.items = 0;
        drop(std::fs::remove_file(&self.path));
        Ok(())
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        drop(std::fs::remove_file(&self.path));
    }
}

#[cfg(test)]
mod tests {
    use crate::feed::InsightsSource;
    use crate::seg::SegResult;
    use crate::seg::bounds::Cursor;
    use crate::seg::format::section;
    use crate::seg::read::Store;
    use crate::{CodeStat, CommitRecord, FileStat};

    use super::super::testing::write_cache;
    use super::super::{DiskSource, parse};
    use super::{ITEM, decode};

    /// Extreme raw values: the column must hold exactly what the row holds, no clamp.
    fn records() -> Vec<CommitRecord> {
        let times = [i64::MIN, -1, 0, 1_700_000_000, i64::MAX];
        let offsets = [i32::MIN, -50_400, 0, 50_400, i32::MAX];
        (0..25usize)
            .map(|i| CommitRecord {
                id: format!("{:040x}", i * 31 + 7),
                author: format!("a{}", i % 3),
                email: "e@x".into(),
                time: times[i % 5],
                offset: offsets[(i / 5) % 5],
                merge: i % 4 == 0,
                kind: (i % 2 == 0).then(|| "fix".into()),
                scope: (i % 3 == 0).then(|| "web".into()),
                files: (0..(i % 4))
                    .map(|n| FileStat {
                        path: format!("p{n}").into(),
                        additions: u32::MAX - n as u32,
                        deletions: u32::MAX,
                        rewritten: 7,
                        code: (n == 1).then_some(CodeStat {
                            additions: 1,
                            deletions: 2,
                            rewritten: 3,
                        }),
                        ..FileStat::default()
                    })
                    .collect(),
                ..CommitRecord::default()
            })
            .collect()
    }

    /// Every item of section 1 equals the record of the same ordinal in `ROWS`, field by
    /// field, raw: times at `i64::MIN` and MAX, offsets at `i32::MIN` and MAX, sums over u32.
    #[test]
    fn columns_equal_rows_field_by_field() -> SegResult<()> {
        let recs = records();
        let dir = tempfile::tempdir()?;
        write_cache(dir.path(), &recs, &[10])?;
        let store =
            Store::open(dir.path())?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
        let mut n = 0;
        for seg in &store.segments {
            let mut cols = Vec::new();
            let mut buf = Vec::new();
            for e in seg.blocks(section::COMMITS) {
                let b = seg.read_block(e, &mut buf)?;
                assert_eq!(b.len(), e.items as usize * ITEM);
                let mut c = Cursor::new(b, seg.name(), e.offset);
                for _ in 0..e.items {
                    cols.push(decode(&mut c)?);
                }
            }
            let mut rows = Vec::new();
            for e in seg.blocks(section::ROWS) {
                rows.extend_from_slice(seg.read_block(e, &mut buf)?);
            }
            let mut c = Cursor::new(&rows, seg.name(), 0);
            for it in &cols {
                let rec = parse(&mut c, None)?;
                assert!(it.agrees(&rec), "{it:?} vs {rec:?}");
                let r = &recs[n];
                assert_eq!((it.time, it.offset, it.merge), (r.time, r.offset, r.merge));
                let add: u64 = r.files.iter().map(|f| u64::from(f.additions)).sum();
                assert_eq!(it.sums.add, add, "no clamp");
                n += 1;
            }
        }
        assert_eq!(n, recs.len());
        let disk = DiskSource::open(
            Store::open(dir.path())?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?,
            &mut |p| Ok(p.to_vec()),
        )?;
        assert_eq!(disk.commits_in(crate::feed::CommitRange::All).count(), recs.len());
        assert_eq!(disk.unreadable(), None);
        Ok(())
    }
}
