//! [`InsightsSource`] over the commit rows of the segment files: the base
//! that [`crate::feed::SegmentSource`] serves commits, rows and strings from. Every read is a
//! pread of one crc-checked block into a byte-bounded cache, so resident memory is the cache
//! plus small per-file tables, never the history.
//!
//! Identity rule: rows carry the raw `name\0email` id. The mailmap of the shown
//! revision is applied once at open into a per-identity vector, so a `.mailmap` edit never
//! rebuilds. Time is stored raw and clamped with `plausible_time` at read, because
//! its bound moves with the wall clock and must never be baked into the cache.
//!
//! A store error inside a trait call is logged, remembered ([`DiskSource::unreadable`]) and
//! answers as absent. The index maps a remembered error to a rebuild, so
//! a corrupt block never serves wrong data.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::feed::{
    AuthorDayRow, CommitRange, CommitRow, FileRow, Hit, IdentId, InsightsSource, LabelId,
    PathAuthorRow, PathId, Posting, Rank, RenameEdge, Stream, Sums,
};
use crate::seg::bounds::{Cursor, MAX_BLOCK};
use crate::seg::format::{TocEntry, section};
use crate::seg::read::{Container, Store};
use crate::seg::{Oid, SegError, SegResult};
use crate::{CodeStat, CommitRecord, FileStat};

/// Bytes of blocks kept between calls. With the small tables this stays inside the 32 MB
/// budget whatever the history size.
const CACHE_BYTES: usize = 12 << 20;
/// One record may span blocks, but never more than this, so a hostile offset chain cannot
/// make one read walk the whole file.
const MAX_RECORD: usize = 64 << 20;
const OID_ENTRY: usize = 24;

/// Maps raw (name, email) pairs to the shown revision's (its `.mailmap`), one call for all.
pub type IdentMap<'a> =
    &'a mut dyn FnMut(&[(String, String)]) -> std::io::Result<Vec<(String, String)>>;

/// Receives one block: section, item count, payload.
pub type Sink<'a> = dyn FnMut(u16, u32, &[u8]) -> SegResult<()> + 'a;

/// One decoded commit record, raw ids.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rec {
    oid: Oid,
    ident: u32,
    time: i64,
    offset: i32,
    merge: bool,
    kind: Option<u32>,
    scope: Option<u32>,
    parents: Vec<Oid>,
    files: u32,
    sums: Sums,
}

/// A segment's slice of the rank space and where its row blocks start.
#[derive(Debug)]
struct Seg {
    c: Container,
    first: u32,
    n: u32,
    /// Payload offset of each `ROWS` block (concatenated), plus the total at the end.
    row_starts: Vec<u64>,
    /// First ordinal of each `ROW_INDEX` block.
    idx_first: Vec<u32>,
    /// First oid of each `OID_INDEX` block, checked against the block when it is read.
    oid_first: Vec<Oid>,
}

/// Where the string blocks of one section start: (first id, delta, toc index in the delta).
type StrIndex = Vec<(u32, usize, usize)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Seg(usize, usize),
    Str(usize, usize),
}

#[derive(Debug)]
struct Block {
    bytes: Vec<u8>,
    /// Item starts for string blocks.
    starts: Vec<u32>,
}

/// Least recently used blocks under a byte budget.
#[derive(Debug, Default)]
struct Cache {
    tick: u64,
    bytes: usize,
    map: std::collections::HashMap<Key, (u64, Arc<Block>)>,
}

impl Cache {
    fn get(&mut self, k: Key) -> Option<Arc<Block>> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(&k).map(|e| {
            e.0 = t;
            Arc::clone(&e.1)
        })
    }

    fn put(&mut self, k: Key, b: Arc<Block>) {
        let size = b.bytes.len() + b.starts.len() * 4;
        while self.bytes + size > CACHE_BYTES && !self.map.is_empty() {
            let old = self.map.iter().min_by_key(|(_, v)| v.0).map(|(k, _)| *k);
            if let Some(old) = old
                && let Some((_, v)) = self.map.remove(&old)
            {
                self.bytes -= v.bytes.len() + v.starts.len() * 4;
            }
        }
        self.tick += 1;
        self.bytes += size;
        self.map.insert(k, (self.tick, b));
    }
}

/// The disk base. Build it with [`DiskSource::open`].
pub struct DiskSource {
    segs: Vec<Seg>,
    deltas: Vec<Container>,
    strs: [StrIndex; 3],
    counts: [u32; 3],
    /// Raw identity id to (name, email) with the mailmap applied.
    idents: Vec<(Arc<str>, Arc<str>)>,
    labels: Vec<Arc<str>>,
    len: u32,
    latest: i64,
    cache: Mutex<Cache>,
    /// (hash of the path, id), sorted, built on first `find_path`.
    path_hash: OnceLock<Vec<(u32, u32)>>,
    broken: Mutex<Option<String>>,
}

impl DiskSource {
    /// Opens the rows of `store`. `map` gets every raw identity once and returns the shown
    /// revision's pairs in the same order. Every segment must hold rows whose ranks are one
    /// contiguous run, together `0..commits`, else `Unreadable`.
    pub fn open(store: Store, map: IdentMap<'_>) -> SegResult<Self> {
        let Store {
            manifest,
            segments,
            strings,
            ..
        } = store;
        let mut segs = Vec::with_capacity(segments.len());
        let mut next = 0u32;
        let mut buf = Vec::new();
        for c in segments {
            let s = open_seg(c, next, &mut buf)?;
            next = next
                .checked_add(s.n)
                .ok_or_else(|| SegError::unreadable(s.c.name(), 0, "rank overflow"))?;
            segs.push(s);
        }
        if next != manifest.commits {
            return Err(SegError::unreadable(
                "MANIFEST",
                0,
                "segment rows do not cover the commits",
            ));
        }
        let mut strs: [StrIndex; 3] = Default::default();
        let mut counts = [0u32; 3];
        for (k, sec) in [section::PATHS, section::IDENTITIES, section::LABELS]
            .into_iter()
            .enumerate()
        {
            let mut id = 0u32;
            for (d, c) in strings.iter().enumerate() {
                let base = [c.base().paths, c.base().identities, c.base().labels][k];
                if base != id {
                    return Err(SegError::unreadable(c.name(), 0, "string base"));
                }
                let start = c.toc().partition_point(|e| e.section < sec);
                for (i, e) in c.blocks(sec).iter().enumerate() {
                    strs[k].push((id, d, start + i));
                    id = id
                        .checked_add(e.items)
                        .ok_or_else(|| SegError::unreadable(c.name(), 0, "string count"))?;
                }
            }
            counts[k] = id;
        }
        let mut src = Self {
            segs,
            deltas: strings,
            strs,
            counts,
            idents: Vec::new(),
            labels: Vec::new(),
            len: next,
            latest: crate::wall_clock().saturating_add(crate::insights::DAY),
            cache: Mutex::new(Cache::default()),
            path_hash: OnceLock::new(),
            broken: Mutex::new(None),
        };
        let mut raw = Vec::with_capacity(counts[1].min(1 << 20) as usize);
        for id in 0..counts[1] {
            raw.push(src.raw_ident(id)?);
        }
        let mapped = map(&raw).map_err(SegError::Io)?;
        if mapped.len() != raw.len() {
            return Err(SegError::Io(std::io::Error::other(
                "identity map returned the wrong number of pairs",
            )));
        }
        src.idents = mapped
            .into_iter()
            .map(|(n, e)| (Arc::from(n), Arc::from(e)))
            .collect();
        for id in 0..counts[2] {
            let l = src.string(2, id)?;
            src.labels.push(l);
        }
        Ok(src)
    }

    /// The first store error a trait call met, if any. The index rebuilds when set.
    pub fn unreadable(&self) -> Option<String> {
        self.broken.lock().ok().and_then(|b| b.clone())
    }

    /// One record as stored (raw identity, raw time), for the segment merge.
    pub fn record_by_oid(&self, oid: &Oid) -> SegResult<Option<CommitRecord>> {
        self.rank_of(oid)?.map(|r| self.record(r)).transpose()
    }

    /// Every record in rank order, raw, for a sequential merge.
    pub fn records(&self) -> impl Iterator<Item = SegResult<CommitRecord>> + '_ {
        (0..self.len).map(|r| self.record(Rank(r)))
    }

    fn record(&self, rank: Rank) -> SegResult<CommitRecord> {
        let mut files = Vec::new();
        let rec = self.decode(rank, Some(&mut files))?;
        let who = self.raw_ident(rec.ident)?;
        let label = |l: Option<u32>| {
            l.map(|i| self.string(2, i).map(|s| s.to_string()))
                .transpose()
        };
        let path = |p: u32| self.string(0, p);
        let mut stats = Vec::new();
        for f in files {
            stats.push(FileStat {
                path: path(f.path.0)?,
                additions: f.add,
                deletions: f.del,
                lines: f.lines,
                rewritten: f.rewritten,
                code: f.code.map(|(a, d, r)| CodeStat {
                    additions: a,
                    deletions: d,
                    rewritten: r,
                }),
                old: f.old.map(|o| path(o.0)).transpose()?,
                binary: f.binary,
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

    fn fail<T>(&self, e: &SegError) -> Option<T> {
        report(e);
        if let Ok(mut b) = self.broken.lock()
            && b.is_none()
        {
            *b = Some(e.to_string());
        }
        None
    }

    fn ok<T>(&self, r: SegResult<T>) -> Option<T> {
        r.map_or_else(|e| self.fail(&e), Some)
    }

    fn block(&self, key: Key) -> SegResult<Arc<Block>> {
        let poisoned = || SegError::Io(std::io::Error::other("block cache lock poisoned"));
        let hit = self.cache.lock().map_err(|_| poisoned())?.get(key);
        if let Some(b) = hit {
            return Ok(b);
        }
        let (c, e, strings) = match key {
            Key::Seg(s, i) => {
                let c = &self.segs[s].c;
                (c, c.toc()[i], false)
            }
            Key::Str(d, i) => {
                let c = &self.deltas[d];
                (c, c.toc()[i], true)
            }
        };
        let mut bytes = Vec::new();
        c.read_block(&e, &mut bytes)?;
        let mut starts = Vec::new();
        if strings {
            let mut cur = Cursor::new(&bytes, c.name(), e.offset);
            let mut at = 0u32;
            for _ in 0..e.items {
                starts.push(at);
                let n = cur.u16()?;
                cur.bytes(usize::from(n))?;
                at = at.saturating_add(2 + u32::from(n));
            }
            if !cur.is_empty() {
                return Err(cur.err("trailing bytes in string block"));
            }
        }
        let b = Arc::new(Block { bytes, starts });
        self.cache
            .lock()
            .map_err(|_| poisoned())?
            .put(key, Arc::clone(&b));
        Ok(b)
    }

    /// String `id` of `kind` (0 paths, 1 identities, 2 labels).
    fn string(&self, kind: usize, id: u32) -> SegResult<Arc<str>> {
        let idx = &self.strs[kind];
        let at = idx.partition_point(|e| e.0 <= id);
        let &(first, delta, toc) = at
            .checked_sub(1)
            .and_then(|i| idx.get(i))
            .ok_or_else(|| SegError::unreadable("strings", 0, "string id out of range"))?;
        if id >= self.counts[kind] {
            return Err(SegError::unreadable("strings", 0, "string id out of range"));
        }
        let name = self.deltas[delta].name();
        let block = self.block(Key::Str(delta, toc))?;
        let start = block
            .starts
            .get((id - first) as usize)
            .ok_or_else(|| SegError::unreadable(name, 0, "string id past block"))?;
        // The starts were built from this block's bytes, so the ranges are inside it.
        let start = *start as usize;
        let len = usize::from(u16::from_le_bytes([
            block.bytes[start],
            block.bytes[start + 1],
        ]));
        let text = std::str::from_utf8(&block.bytes[start + 2..start + 2 + len])
            .map_err(|_| SegError::unreadable(name, 0, "string not utf-8"))?;
        Ok(Arc::from(text))
    }

    fn raw_ident(&self, id: u32) -> SegResult<(String, String)> {
        let s = self.string(1, id)?;
        let mut parts = s.split('\0');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(n), Some(e), None) => Ok((n.to_owned(), e.to_owned())),
            _ => Err(SegError::unreadable(
                "strings",
                0,
                "identity is not name NUL email",
            )),
        }
    }

    fn seg_of(&self, rank: Rank) -> Option<(usize, u32)> {
        let i = self
            .segs
            .partition_point(|s| s.first <= rank.0)
            .checked_sub(1)?;
        let s = &self.segs[i];
        (rank.0 - s.first < s.n).then_some((i, rank.0 - s.first))
    }

    /// Byte offset of ordinal `o` in segment `s`'s row payload, from a crc-checked block.
    fn row_offset(&self, s: usize, o: u32) -> SegResult<u64> {
        let seg = &self.segs[s];
        let bi = seg.idx_first.partition_point(|&f| f <= o) - 1;
        let start = seg
            .c
            .toc()
            .partition_point(|e| e.section < section::ROW_INDEX);
        let b = self.block(Key::Seg(s, start + bi))?;
        let at = (o - seg.idx_first[bi]) as usize * 8;
        let bytes = b
            .bytes
            .get(at..at + 8)
            .ok_or_else(|| SegError::unreadable(seg.c.name(), 0, "row index short"))?;
        let mut a = [0; 8];
        a.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(a))
    }

    /// Decodes the record of `rank`. File rows go to `files` when given.
    fn decode(&self, rank: Rank, files: Option<&mut Vec<FileRow>>) -> SegResult<Rec> {
        let (s, o) = self
            .seg_of(rank)
            .ok_or_else(|| SegError::unreadable("rows", 0, "rank out of range"))?;
        let seg = &self.segs[s];
        let name = seg.c.name();
        let at = self.row_offset(s, o)?;
        let end = if o + 1 < seg.n {
            self.row_offset(s, o + 1)?
        } else {
            *seg.row_starts.last().unwrap_or(&0)
        };
        if end <= at || end - at > MAX_RECORD as u64 {
            return Err(SegError::unreadable(name, at, "row offsets"));
        }
        let first_block = seg.row_starts.partition_point(|&p| p <= at) - 1;
        let toc0 = seg.c.toc().partition_point(|e| e.section < section::ROWS);
        let mut bi = first_block;
        let mut joined: Vec<u8> = Vec::new();
        let one;
        let bytes: &[u8] = loop {
            let b = self.block(Key::Seg(s, toc0 + bi))?;
            let bstart = seg.row_starts[bi];
            let from = usize::try_from(at.saturating_sub(bstart)).unwrap_or(usize::MAX);
            let to = usize::try_from((end - bstart).min(b.bytes.len() as u64)).unwrap_or(0);
            if bi == first_block && end <= bstart + b.bytes.len() as u64 {
                one = b;
                break &one.bytes[from..to];
            }
            joined.extend_from_slice(b.bytes.get(from..to).unwrap_or_default());
            if joined.len() as u64 >= end - at {
                break &joined;
            }
            bi += 1;
            if bi + 1 >= seg.row_starts.len() {
                return Err(SegError::unreadable(name, at, "record past the rows"));
            }
        };
        let mut c = Cursor::new(bytes, name, at);
        let rec = parse(&mut c, files)?;
        if !c.is_empty() {
            return Err(c.err("record shorter than its index span"));
        }
        Ok(rec)
    }

    fn row(&self, rank: Rank) -> SegResult<CommitRow> {
        let r = self.decode(rank, None)?;
        Ok(CommitRow {
            rank,
            ident: IdentId(r.ident),
            time: crate::plausible_time(r.time, self.latest),
            offset: r.offset,
            merge: r.merge,
            kind: r.kind.map(LabelId),
            scope: r.scope.map(LabelId),
            files: r.files,
            sums: r.sums,
        })
    }

    fn rank_of(&self, oid: &Oid) -> SegResult<Option<Rank>> {
        for (s, seg) in self.segs.iter().enumerate() {
            let bi = seg.oid_first.partition_point(|f| f <= oid);
            let Some(bi) = bi.checked_sub(1) else {
                continue;
            };
            let start = seg
                .c
                .toc()
                .partition_point(|e| e.section < section::OID_INDEX);
            let b = self.block(Key::Seg(s, start + bi))?;
            let n = b.bytes.len() / OID_ENTRY;
            let at = |i: usize| &b.bytes[i * OID_ENTRY..i * OID_ENTRY + 20];
            if n == 0 || at(0) != seg.oid_first[bi] {
                return Err(SegError::unreadable(seg.c.name(), 0, "oid index routing"));
            }
            let (mut lo, mut hi) = (0, n);
            while lo < hi {
                let mid = usize::midpoint(lo, hi);
                match at(mid).cmp(oid.as_slice()) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => {
                        let e = &b.bytes[mid * OID_ENTRY + 20..mid * OID_ENTRY + 24];
                        let o = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
                        if o >= seg.n {
                            return Err(SegError::unreadable(seg.c.name(), 0, "oid ordinal"));
                        }
                        return Ok(Some(Rank(seg.first + o)));
                    }
                }
            }
        }
        Ok(None)
    }

    fn path_index(&self) -> SegResult<Vec<(u32, u32)>> {
        let mut v = Vec::with_capacity(self.counts[0].min(1 << 22) as usize);
        for id in 0..self.counts[0] {
            v.push((hash32(&self.string(0, id)?), id));
        }
        v.sort_unstable();
        Ok(v)
    }

    /// Every file row with its rank, for the aggregates this source answers on its own (the
    /// segments answer them when this source is wrapped).
    fn each_row(&self, mut f: impl FnMut(&CommitRow, &[FileRow])) {
        let mut files = Vec::new();
        for r in 0..self.len {
            files.clear();
            let Some(rec) = self.ok(self.decode(Rank(r), Some(&mut files))) else {
                return;
            };
            let row = CommitRow {
                rank: Rank(r),
                ident: IdentId(rec.ident),
                time: crate::plausible_time(rec.time, self.latest),
                offset: rec.offset,
                merge: rec.merge,
                kind: rec.kind.map(LabelId),
                scope: rec.scope.map(LabelId),
                files: rec.files,
                sums: rec.sums,
            };
            f(&row, &files);
        }
    }
}

fn open_seg(c: Container, first_expected: u32, buf: &mut Vec<u8>) -> SegResult<Seg> {
    let total = |sec| {
        c.blocks(sec)
            .iter()
            .map(|e| u64::from(e.items))
            .sum::<u64>()
    };
    let n = total(section::RANK);
    let bad = |what| SegError::unreadable(c.name(), 0, what);
    if total(section::ROWS) != n || total(section::ROW_INDEX) != n || total(section::OID_INDEX) != n
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
    let mut at = 0u32;
    for e in c.blocks(section::OID_INDEX) {
        if !fixed(e, 24) || e.items == 0 {
            return Err(bad("oid index block length"));
        }
        // Routing only: the block's crc is checked when it is searched, and its first oid
        // must equal this one then.
        let mut o = [0u8; OID_ENTRY];
        c.record_at(section::OID_INDEX, at, OID_ENTRY, &mut o)?;
        let mut oid = [0u8; 20];
        oid.copy_from_slice(&o[..20]);
        oid_first.push(oid);
        at += e.items;
    }
    if !oid_first.is_sorted() {
        return Err(bad("oid index not sorted"));
    }
    Ok(Seg {
        c,
        first: first_expected,
        n,
        row_starts,
        idx_first,
        oid_first,
    })
}

/// Parses one record. File rows are pushed one at a time, never sized by the count: a
/// hostile count with a valid crc must fail at the end of the bytes, not allocate for it.
fn parse(c: &mut Cursor<'_>, mut files: Option<&mut Vec<FileRow>>) -> SegResult<Rec> {
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
        if ff > 3 {
            return Err(c.err("file flags"));
        }
        let code = if ff & 2 != 0 {
            Some((c.u32()?, c.u32()?, c.u32()?))
        } else {
            None
        };
        let f = FileRow {
            path,
            old,
            add,
            del,
            lines,
            rewritten,
            code,
            binary: ff & 1 != 0,
        };
        sums.add_row(&f);
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

fn encode(out: &mut Vec<u8>, r: &CommitRecord, w: &Ids<'_>) -> SegResult<()> {
    let oid = oid(&r.id)?;
    out.extend_from_slice(&oid);
    out.extend_from_slice(&w.who.to_le_bytes());
    out.extend_from_slice(&r.time.to_le_bytes());
    out.extend_from_slice(&r.offset.to_le_bytes());
    out.push(u8::from(r.merge));
    let opt = |v: Option<u32>| v.map_or(0, |v| v + 1);
    out.extend_from_slice(&opt(w.kind).to_le_bytes());
    out.extend_from_slice(&opt(w.scope).to_le_bytes());
    let np = u16::try_from(r.parents.len()).map_err(|_| SegError::TooLong {
        kind: "parent list",
        len: r.parents.len(),
        max: u16::MAX as usize,
    })?;
    out.extend_from_slice(&np.to_le_bytes());
    for p in &r.parents {
        out.extend_from_slice(&oid_of(p)?);
    }
    if w.paths.len() != r.files.len() || w.olds.len() != r.files.len() {
        return Err(SegError::BadName("row ids do not match the files".into()));
    }
    let nf = u32::try_from(r.files.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&nf.to_le_bytes());
    for ((f, &p), &o) in r.files.iter().zip(w.paths).zip(w.olds) {
        for v in [p, opt(o), f.additions, f.deletions, f.lines, f.rewritten] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(u8::from(f.binary) | (u8::from(f.code.is_some()) << 1));
        if let Some(c) = &f.code {
            for v in [c.additions, c.deletions, c.rewritten] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    Ok(())
}

struct Ids<'a> {
    who: u32,
    kind: Option<u32>,
    scope: Option<u32>,
    paths: &'a [u32],
    olds: &'a [Option<u32>],
}

fn oid(id: &str) -> SegResult<Oid> {
    oid_of(id)
}

fn oid_of(id: &str) -> SegResult<Oid> {
    crate::store::oid_of(id).ok_or_else(|| SegError::BadName(format!("commit id {id:?}")))
}

fn hex(oid: &Oid) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(40);
    for b in oid {
        s.push(char::from(H[usize::from(b >> 4)]));
        s.push(char::from(H[usize::from(b & 15)]));
    }
    s
}

fn hash32(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in s.as_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache error goes to the user's stderr"
)]
fn report(e: &SegError) {
    eprintln!("gitterrace: {e}");
}

/// Writes the row sections of one segment.
///
/// Records are spooled to `rows-<gen>.gts.tmp` in
/// the cache directory while the other sections are built (sections ascend in a container),
/// then copied into blocks by [`RowWriter::finish`]. A crash leaves only the `.tmp`, which
/// the open sweep removes, and the spool is never named in the manifest.
pub struct RowWriter {
    path: PathBuf,
    spool: BufWriter<File>,
    offsets: Vec<u64>,
    oids: Vec<(Oid, u32)>,
    len: u64,
    buf: Vec<u8>,
}

impl RowWriter {
    pub fn new(dir: &Path, generation: u64) -> SegResult<Self> {
        let path = dir.join(format!("rows-{generation}.gts.tmp"));
        let spool = BufWriter::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?,
        );
        Ok(Self {
            path,
            spool,
            offsets: Vec::new(),
            oids: Vec::new(),
            len: 0,
            buf: Vec::new(),
        })
    }

    /// Appends the next record (the next local ordinal). Ids are the cache's global ids, one
    /// path and old path per file row. A NUL in the author or email is refused.
    pub fn push(
        &mut self,
        r: &CommitRecord,
        who: u32,
        kind: Option<u32>,
        scope: Option<u32>,
        paths: &[u32],
        olds: &[Option<u32>],
    ) -> SegResult<()> {
        if r.author.contains('\0') || r.email.contains('\0') {
            return Err(SegError::BadName("NUL in an author name or email".into()));
        }
        self.buf.clear();
        let ids = Ids {
            who,
            kind,
            scope,
            paths,
            olds,
        };
        encode(&mut self.buf, r, &ids)?;
        if self.buf.len() > MAX_RECORD {
            return Err(SegError::TooLong {
                kind: "commit record",
                len: self.buf.len(),
                max: MAX_RECORD,
            });
        }
        let ord = u32::try_from(self.offsets.len()).unwrap_or(u32::MAX);
        self.oids.push((oid(&r.id)?, ord));
        self.offsets.push(self.len);
        self.spool.write_all(&self.buf)?;
        self.len += self.buf.len() as u64;
        Ok(())
    }

    /// Writes sections 34, 35 and 36 through `sink`, after section 33. Removes the spool.
    pub fn finish(self, sink: &mut Sink<'_>) -> SegResult<()> {
        let Self {
            path,
            spool,
            offsets,
            mut oids,
            len,
            ..
        } = self;
        let mut file = spool
            .into_inner()
            .map_err(|e| SegError::Io(e.into_error()))?;
        std::io::Seek::rewind(&mut file)?;
        let mut block = vec![0u8; MAX_BLOCK];
        let (mut pos, mut next) = (0u64, 0usize);
        while pos < len {
            let n = usize::try_from((len - pos).min(MAX_BLOCK as u64)).unwrap_or(MAX_BLOCK);
            file.read_exact(&mut block[..n])?;
            let end = pos + n as u64;
            let mut items = 0u32;
            while next < offsets.len() && offsets[next] < end {
                items += 1;
                next += 1;
            }
            sink(section::ROWS, items, &block[..n])?;
            pos = end;
        }
        drop(file);
        drop(std::fs::remove_file(&path));
        for chunk in offsets.chunks(MAX_BLOCK / 8) {
            let bytes: Vec<u8> = chunk.iter().flat_map(|o| o.to_le_bytes()).collect();
            sink(
                section::ROW_INDEX,
                u32::try_from(chunk.len()).unwrap_or(0),
                &bytes,
            )?;
        }
        oids.sort_unstable();
        for chunk in oids.chunks(MAX_BLOCK / OID_ENTRY) {
            let mut bytes = Vec::with_capacity(chunk.len() * OID_ENTRY);
            for (o, ord) in chunk {
                bytes.extend_from_slice(o);
                bytes.extend_from_slice(&ord.to_le_bytes());
            }
            sink(
                section::OID_INDEX,
                u32::try_from(chunk.len()).unwrap_or(0),
                &bytes,
            )?;
        }
        Ok(())
    }
}

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
            ranks
                .map_while(move |r| self.commit(Rank(r)))
                .filter(move |c| time.as_ref().is_none_or(|t| t.contains(&c.time))),
        )
    }

    fn rows_for(&self, commits: Vec<Rank>) -> Stream<'_, (Rank, Vec<FileRow>)> {
        Box::new(commits.into_iter().filter_map(move |r| {
            if r.0 >= self.len {
                return None;
            }
            let mut files = Vec::new();
            self.ok(self.decode(r, Some(&mut files)))
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

/// Writes a segment cache from records the way the build does, for this crate's guards and
/// the memory guard in `tests/disk_memory.rs`. Not an API.
#[doc(hidden)]
pub mod testing {
    use std::path::Path;

    use crate::CommitRecord;
    use crate::feed::RowWriter;
    use crate::feed::segments::SegmentBuilder;
    use crate::seg::SegResult;
    use crate::seg::agg::{Interner, Strings};
    use crate::seg::format::{Kind, StringBase};
    use crate::seg::manifest::Manifest;
    use crate::seg::strings::{StringDelta, write_delta};
    use crate::seg::write::{ContainerWriter, write_order};

    /// Writes `recs` as segments split at `cuts` (an append-style set), as the build does:
    /// global string ids, local ordinals, ranks `old_len + local`.
    pub fn write_cache(dir: &Path, recs: &[CommitRecord], cuts: &[usize]) -> SegResult<()> {
        let mut s = Strings::default();
        let mut labels = Interner::default();
        let mut base = StringBase::default();
        let mut live = Vec::new();
        let mut bounds = vec![0];
        bounds.extend_from_slice(cuts);
        bounds.push(recs.len());
        for (n, w) in bounds.windows(2).enumerate() {
            let spill = dir.join(format!("spill-t{n}.gts.tmp"));
            let mut b = SegmentBuilder::new(move || {
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&spill)
            });
            let mut rows = RowWriter::new(dir, n as u64 + 1)?;
            for (i, r) in recs.iter().enumerate().take(w[1]).skip(w[0]) {
                b.push(&mut s, r, i as u32)?;
                let (who, paths) = s.ids(r);
                let olds: Vec<Option<u32>> = r
                    .files
                    .iter()
                    .map(|f| f.old.as_deref().and_then(|o| s.paths.get(o)))
                    .collect();
                let kind = r.kind.as_deref().map(|k| labels.id(k));
                let scope = r.scope.as_deref().map(|k| labels.id(k));
                rows.push(r, who, kind, scope, &paths, &olds)?;
            }
            let name = format!("seg-1-{n}.gts");
            let mut cw = ContainerWriter::new(dir, &name, Kind::Segment, base)?;
            b.finish(&mut |sec, items, p| cw.block(sec, items, p))?;
            rows.finish(&mut |sec, items, p| cw.block(sec, items, p))?;
            live.push(cw.finish()?);
            let tail = |l: &[Box<str>], from: u32| -> Vec<String> {
                l[from as usize..].iter().map(ToString::to_string).collect()
            };
            let delta = StringDelta {
                paths: tail(&s.paths.list, base.paths),
                identities: tail(&s.identities.list, base.identities),
                labels: tail(&labels.list, base.labels),
            };
            let (e, next) = write_delta(dir, &format!("str-1-{n}.gts"), base, &delta)?;
            live.push(e);
            base = next;
        }
        let oids: Vec<_> = recs
            .iter()
            .filter_map(|r| crate::store::oid_of(&r.id))
            .collect();
        live.push(write_order(dir, "ord-1.gts", &oids)?);
        let mut m = Manifest {
            generation: 1,
            fingerprint: crate::seg::fingerprint(&oids),
            commits: recs.len() as u32,
            shallow: 0,
            live,
            retired: vec![],
        };
        m.commit(dir)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::DiskSource;
    use crate::feed::{
        CommitRange, IdentId, IdentPairs, InsightsSource, LabelId, MemorySource, PathId, Posting,
        Rank,
    };
    use crate::seg::SegResult;
    use crate::seg::read::Store;
    use crate::{CodeStat, CommitRecord, FileStat};

    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn oid(i: usize) -> String {
        format!("{:040x}", i * 7919 + 1)
    }

    fn f(path: &str, add: u32, del: u32) -> FileStat {
        FileStat {
            path: path.into(),
            additions: add,
            deletions: del,
            lines: add * 3,
            rewritten: add.min(del),
            ..FileStat::default()
        }
    }

    /// Every field the rows carry: merges, renames, code triples, binaries, labels,
    /// several parents, two raw identities the mailmap joins, an odd timezone, a far
    /// future date (`plausible_time` at read) and an empty commit.
    fn fixture() -> Vec<CommitRecord> {
        let mut v = Vec::new();
        for i in 0..40usize {
            let who = ["Ann", "ann", "Bob", "Cy"][i % 4];
            let mut files = vec![f(&format!("src/m{}.rs", i % 7), i as u32, (i % 3) as u32)];
            if i % 5 == 0 {
                files.push(FileStat {
                    old: Some(format!("old/{}.rs", i % 4).into()),
                    code: Some(CodeStat {
                        additions: 1,
                        deletions: 0,
                        rewritten: 0,
                    }),
                    ..f(&format!("new/{i}.rs"), 2, 2)
                });
            }
            if i % 9 == 0 {
                files.push(FileStat {
                    binary: true,
                    ..f("img.png", 0, 0)
                });
            }
            if i == 13 {
                files.clear();
            }
            v.push(CommitRecord {
                id: oid(i),
                author: who.into(),
                email: if who == "ann" {
                    "ANN@x.io".into()
                } else {
                    format!("{who}@x.io")
                },
                time: if i == 30 {
                    4_000_000_000
                } else {
                    1_700_000_000 + (i as i64) * 50_000
                },
                offset: if i % 2 == 0 { 3600 } else { -19_800 },
                merge: i % 11 == 10,
                parents: (0..(i.min(1) + usize::from(i % 11 == 10)))
                    .map(|k| oid(i.saturating_sub(k + 1)))
                    .collect(),
                kind: (i % 3 == 0).then(|| ["feat", "fix"][i % 2].into()),
                scope: (i % 6 == 0).then(|| "web".into()),
                files,
            });
        }
        v
    }

    fn mailmap(recs: &[CommitRecord]) -> IdentPairs {
        recs.iter()
            .map(|r| {
                let raw = (r.author.clone(), r.email.clone());
                let shown = if r.email.eq_ignore_ascii_case("ann@x.io") {
                    ("Ann".to_owned(), "ann@x.io".to_owned())
                } else {
                    raw.clone()
                };
                (raw, shown)
            })
            .collect()
    }

    use super::testing::write_cache;

    fn open(dir: &Path, map: &IdentPairs) -> SegResult<DiskSource> {
        let store =
            Store::open(dir)?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
        DiskSource::open(store, &mut |pairs| {
            Ok(pairs
                .iter()
                .map(|p| map.get(p).cloned().unwrap_or_else(|| p.clone()))
                .collect())
        })
    }

    /// Every trait method, both sides, over every id and rank plus out-of-range ones.
    #[expect(
        clippy::many_single_char_names,
        reason = "d and m are the two sides throughout"
    )]
    fn assert_same(d: &dyn InsightsSource, m: &dyn InsightsSource, case: &str) {
        assert_eq!(d.len(), m.len(), "{case}: len");
        let n = m.len() as u32;
        for r in 0..n + 2 {
            let r = Rank(r);
            assert_eq!(d.commit(r), m.commit(r), "{case}: commit {r:?}");
            assert_eq!(d.commit_id(r), m.commit_id(r), "{case}: id {r:?}");
            assert_eq!(d.parent_ids(r), m.parent_ids(r), "{case}: parents {r:?}");
            if let Some(id) = m.commit_id(r) {
                assert_eq!(d.find_commit(&id), m.find_commit(&id), "{case}: find {id}");
            }
        }
        assert_eq!(d.find_commit(&oid(99_999)), None, "{case}: absent commit");
        for range in [
            CommitRange::All,
            CommitRange::Ranks(3..17),
            CommitRange::Ranks(n.saturating_sub(2)..n + 5),
            CommitRange::Time(1_700_100_000..1_700_900_000),
        ] {
            let a: Vec<_> = d.commits_in(range.clone()).collect();
            let b: Vec<_> = m.commits_in(range.clone()).collect();
            assert_eq!(a, b, "{case}: commits_in {range:?}");
        }
        let ranks: Vec<Rank> = (0..=n).map(Rank).collect();
        let a: Vec<_> = d.rows_for(ranks.clone()).collect();
        let b: Vec<_> = m.rows_for(ranks).collect();
        assert_eq!(a, b, "{case}: rows_for");
        for days in [i64::MIN..i64::MAX, 19_680..19_700] {
            let a: Vec<_> = d.agg_author_day(days.clone()).collect();
            let b: Vec<_> = m.agg_author_day(days.clone()).collect();
            assert_eq!(a, b, "{case}: author day");
        }
        for p in ["", "src", "new/", "zz"] {
            let a: Vec<_> = d.agg_path_author(p).collect();
            let b: Vec<_> = m.agg_path_author(p).collect();
            assert_eq!(a, b, "{case}: path author {p}");
        }
        assert_eq!(d.paths(), m.paths(), "{case}: paths");
        for p in 0..m.paths() as u32 {
            let s = m.path(PathId(p));
            assert_eq!(d.path(PathId(p)), s, "{case}: path {p}");
            assert_eq!(
                s.and_then(|s| d.find_path(&s)),
                Some(PathId(p)),
                "{case}: find {p}"
            );
        }
        // Postings are a full scan here (served from segments in production), so probe the
        // first 64 paths and the last, not 60k of the wide case.
        let np = m.paths() as u32;
        for p in (0..np.min(64)).chain([np.saturating_sub(1), np]) {
            let id = PathId(p);
            assert_eq!(d.path(id), m.path(id), "{case}: path {p}");
            if let Some(s) = m.path(id) {
                assert_eq!(d.find_path(&s), m.find_path(&s), "{case}: find path {s}");
            }
            let a: Vec<_> = d.postings(Posting::Path(id)).collect();
            let b: Vec<_> = m.postings(Posting::Path(id)).collect();
            assert_eq!(a, b, "{case}: path postings {p}");
        }
        assert_eq!(
            d.find_path("nope"),
            m.find_path("nope"),
            "{case}: absent path"
        );
        assert_eq!(d.idents(), m.idents(), "{case}: idents");
        for i in 0..=m.idents() as u32 {
            assert_eq!(
                d.ident(IdentId(i)),
                m.ident(IdentId(i)),
                "{case}: ident {i}"
            );
            let a: Vec<_> = d.postings(Posting::Author(IdentId(i))).collect();
            let b: Vec<_> = m.postings(Posting::Author(IdentId(i))).collect();
            assert_eq!(a, b, "{case}: author postings {i}");
        }
        for l in 0..4 {
            let id = LabelId(l);
            assert_eq!(d.label(id), m.label(id), "{case}: label {l}");
            if let Some(s) = m.label(id) {
                assert_eq!(d.find_label(&s), m.find_label(&s), "{case}: find label {s}");
            }
        }
        assert_eq!(
            d.find_label("chore"),
            m.find_label("chore"),
            "{case}: absent label"
        );
        let a: Vec<_> = d.rename_edges().collect();
        let b: Vec<_> = m.rename_edges().collect();
        assert_eq!(a, b, "{case}: rename edges");
    }

    /// `DiskSource` equals `MemorySource` (raw identities plus the mailmap, the one rule) on
    /// every trait method, over the option matrix the source can see: mailmap as is and
    /// edited, one segment and an unmerged append set, a record over the block size, and
    /// a seeded set of random histories. Options (bots, filters, whitespace, period) apply
    /// above the source to the raw facts compared here.
    #[test]
    fn disk_equals_memory_on_every_method() -> SegResult<()> {
        let _one = ONE_AT_A_TIME.lock();
        let mut cases: Vec<(String, Vec<CommitRecord>)> = vec![("fixture".into(), fixture())];
        // A commit of 60k files spans two 1 MiB row blocks (the join path).
        let mut big = fixture();
        big[20].files = (0..60_000).map(|k| f(&format!("wide/{k}"), 1, 0)).collect();
        cases.push(("big record".into(), big));
        let big_only = |name: &str, edited: bool, cuts: &[usize]| {
            name != "big record" || (!edited && cuts.len() == 1)
        };
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for k in 0..6 {
            let mut v = fixture();
            for r in &mut v {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                r.offset = (seed % 50_000) as i32 - 25_000;
                r.files.truncate((seed % 4) as usize);
                if seed.is_multiple_of(5) {
                    r.kind = None;
                }
            }
            cases.push((format!("random {k}"), v));
        }
        for (name, recs) in cases {
            for edited in [false, true] {
                let map = if edited {
                    mailmap(&recs)
                } else {
                    IdentPairs::new()
                };
                // Index::records hands the oracle clamped times, the disk clamps at read.
                let latest = 1_800_000_000;
                let mut shown = recs.clone();
                for r in &mut shown {
                    r.time = crate::plausible_time(r.time, latest);
                }
                let mem = MemorySource::shared_raw(Arc::new(shown), map.clone());
                for cuts in [vec![], vec![17], vec![5, 30]] {
                    if !big_only(&name, edited, &cuts) {
                        continue;
                    }
                    let dir = tempfile::tempdir()?;
                    write_cache(dir.path(), &recs, &cuts)?;
                    let mut disk = open(dir.path(), &map)?;
                    disk.latest = latest;
                    let case = format!("{name}, mailmap edited {edited}, cuts {cuts:?}");
                    assert_same(&disk, &mem, &case);
                    assert_eq!(disk.unreadable(), None, "{case}");
                    let back: Vec<CommitRecord> = disk.records().collect::<SegResult<_>>()?;
                    for (a, b) in back.iter().zip(&recs) {
                        assert_eq!(format!("{a:?}"), format!("{b:?}"), "{case}: record");
                    }
                    let probe = crate::store::oid_of(&recs[7].id).unwrap_or_default();
                    let got = disk.record_by_oid(&probe)?.map(|r| r.id);
                    assert_eq!(got.as_deref(), Some(recs[7].id.as_str()), "{case}");
                }
            }
        }
        // The rule itself: joined raw identities keep two ids with equal strings.
        let recs = fixture();
        let mem = MemorySource::shared_raw(Arc::new(recs.clone()), mailmap(&recs));
        assert_eq!(mem.ident(IdentId(0)), mem.ident(IdentId(1)));
        Ok(())
    }

    fn flip_in_rows(dir: &Path) -> SegResult<()> {
        let store =
            Store::open(dir)?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
        let seg = &store.segments[0];
        let e = seg.blocks(crate::seg::format::section::ROWS)[0];
        let path = dir.join(seg.name());
        let mut bytes = std::fs::read(&path)?;
        // Byte 24 of the first record is the low byte of its time.
        bytes[e.offset as usize + 24] ^= 1;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// A flipped byte in a row block reads as absent plus `unreadable()` (the index then
    /// rebuilds), never as a commit with a wrong value.
    #[test]
    fn corrupt_row_block_is_unreadable_never_wrong() -> SegResult<()> {
        let _one = ONE_AT_A_TIME.lock();
        let recs = fixture();
        let dir = tempfile::tempdir()?;
        write_cache(dir.path(), &recs, &[])?;
        flip_in_rows(dir.path())?;
        let disk = open(dir.path(), &IdentPairs::new())?;
        assert_eq!(disk.unreadable(), None);
        assert_eq!(disk.commit(Rank(0)), None);
        assert!(disk.unreadable().is_some_and(|e| e.contains("crc")));
        assert_eq!(disk.rows_for(vec![Rank(0)]).count(), 0);
        assert!(disk.commits_in(CommitRange::All).next().is_none());
        Ok(())
    }

    /// A record claiming 2^24 files with a valid crc fails at the end of its bytes (the
    /// allocation cap is in `tests/disk_memory.rs`), and a row index pointing past the rows or
    /// backwards is `Unreadable`.
    #[test]
    fn hostile_counts_and_offsets_are_unreadable() -> SegResult<()> {
        let _one = ONE_AT_A_TIME.lock();
        let recs = fixture();
        for what in ["nfiles", "offset"] {
            let dir = tempfile::tempdir()?;
            write_cache(dir.path(), &recs, &[])?;
            let store = Store::open(dir.path())?
                .ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
            let seg = &store.segments[0];
            let sec = if what == "nfiles" {
                crate::seg::format::section::ROWS
            } else {
                crate::seg::format::section::ROW_INDEX
            };
            let e = seg.blocks(sec)[0];
            let name = seg.name().to_owned();
            let toc_at = seg.toc().iter().position(|t| *t == e).unwrap_or(0);
            drop(store);
            let path = dir.path().join(&name);
            let mut b = std::fs::read(&path)?;
            let at = e.offset as usize;
            if what == "nfiles" {
                // Record 0: oid 20, ident 4, time 8, offset 4, flags 1, kind 4, scope 4,
                // parent count 2 (0 parents), then the file count.
                b[at + 47..at + 51].copy_from_slice(&(1u32 << 24).to_le_bytes());
            } else {
                b[at + 8..at + 16].copy_from_slice(&u64::MAX.to_le_bytes());
            }
            reseal(&mut b, toc_at, at, e.len as usize);
            std::fs::write(&path, &b)?;
            let store = Store::open(dir.path())?
                .ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
            let mut entry = store.manifest.clone();
            drop(store);
            for f in &mut entry.live {
                if f.name == name {
                    f.len = b.len() as u64;
                    f.footer_crc = crc32fast::hash(&b[b.len() - 64..]);
                }
            }
            entry.commit(dir.path())?;
            let disk = open(dir.path(), &IdentPairs::new())?;
            let got = disk.commit(Rank(0));
            let rows = disk.rows_for(vec![Rank(0)]).count();
            assert_eq!((got, rows), (None, 0), "{what}");
            assert!(disk.unreadable().is_some(), "{what}");
        }
        Ok(())
    }

    /// Recomputes a block crc in the toc, the toc crc and the footer crc.
    fn reseal(b: &mut [u8], toc_index: usize, at: usize, len: usize) {
        let n = b.len();
        let foot = n - 64;
        let toc_off = u64::from_le_bytes(b[foot + 16..foot + 24].try_into().unwrap_or_default());
        let toc_count = u32::from_le_bytes(b[foot + 24..foot + 28].try_into().unwrap_or_default());
        let toc = toc_off as usize;
        let crc = crc32fast::hash(&b[at..at + len]);
        let entry = toc + toc_index * 24;
        b[entry + 20..entry + 24].copy_from_slice(&crc.to_le_bytes());
        let toc_crc = crc32fast::hash(&b[toc..toc + toc_count as usize * 24]);
        b[foot + 28..foot + 32].copy_from_slice(&toc_crc.to_le_bytes());
        let fcrc = crc32fast::hash(&b[foot..foot + 60]);
        b[foot + 60..].copy_from_slice(&fcrc.to_le_bytes());
    }
}
