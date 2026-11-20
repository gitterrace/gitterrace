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

mod columns;
mod record;
mod scan;
mod source;
mod stored;
mod writer;

use std::sync::{Arc, Mutex, OnceLock};

use crate::CommitRecord;
use crate::feed::disk::record::{open_seg, parse, report};
use crate::feed::{CommitRow, FileRow, IdentId, LabelId, Rank, Sums};
use crate::seg::bounds::Cursor;
use crate::seg::format::section;
use crate::seg::read::{Container, Store};
use crate::seg::{Oid, SegError, SegResult};

pub use writer::RowWriter;

/// One record may span blocks, but never more than this, so a hostile offset chain cannot
/// make one read walk the whole file.
/// Not configurable: an on-disk format bound against a corrupt or hostile cache.
const MAX_RECORD: usize = 64 << 20;
const OID_ENTRY: usize = 24;

/// Maps raw (name, email) pairs to the shown revision's (its `.mailmap`), one call for all.
pub type IdentMap<'a> =
    &'a mut dyn FnMut(&[(String, String)]) -> std::io::Result<Vec<(String, String)>>;

/// Receives one block: section, item count, payload.
pub type Sink<'a> = dyn FnMut(u16, u32, &[u8]) -> SegResult<()> + 'a;

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
    /// First ordinal of each `COMMITS` (section 1) block.
    col_first: Vec<u32>,
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
    starts: Vec<u32>,
}

#[derive(Debug, Default)]
struct Cache {
    limit: usize,
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
        while self.bytes + size > self.limit && !self.map.is_empty() {
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

/// Bytes of blocks kept between calls: the block share of the default `resources.cache_mb`.
///
/// Flat in history size. A server sets its own share with [`DiskSource::with_cache_bytes`].
/// The cache lives as long as the source, which the server drops on its idle release.
pub fn block_cache_default() -> usize {
    gitterrace_config::Config::default().cache_bytes(gitterrace_config::Cache::Blocks)
}

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
    /// Set when stored sums added past u64 across commits ([`InsightsSource::failure`]).
    pub(super) over: crate::feed::SumOverflow,
    /// See [`InsightsSource::instance`].
    #[cfg(any(test, feature = "oracle"))]
    pub(super) instance: u64,
    /// The stored tip touches when written for this set, see `crate::tip`.
    tip: Option<Container>,
}

impl DiskSource {
    /// Opens the rows of `store`. `map` gets every raw identity once and returns the shown
    /// revision's pairs in the same order. Every segment must hold rows whose ranks are one
    /// contiguous run, together `0..commits`, else `Unreadable`.
    /// Keeps at most `bytes` of blocks between calls.
    #[must_use]
    pub fn with_cache_bytes(self, bytes: usize) -> Self {
        if let Ok(mut c) = self.cache.lock() {
            c.limit = bytes;
        }
        self
    }

    pub fn open(store: Store, map: IdentMap<'_>) -> SegResult<Self> {
        let Store { manifest, segments, strings, tip, .. } = store;
        let tip = match tip {
            Some(c) => crate::tip::current(&c, &manifest)?.map(|_| c),
            None => None,
        };
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
        for (k, sec) in
            [section::PATHS, section::IDENTITIES, section::LABELS].into_iter().enumerate()
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
            cache: Mutex::new(Cache { limit: block_cache_default(), ..Cache::default() }),
            path_hash: OnceLock::new(),
            broken: Mutex::new(None),
            over: crate::feed::SumOverflow::new(),
            #[cfg(any(test, feature = "oracle"))]
            instance: crate::feed::next_instance(),
            tip,
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
        src.idents = mapped.into_iter().map(|(n, e)| (Arc::from(n), Arc::from(e))).collect();
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

    /// [`DiskSource::ok`] for a finder: the error is remembered and returned, never absence.
    fn checked<T>(&self, r: SegResult<T>) -> Result<T, crate::feed::SourceError> {
        r.map_err(|e| {
            let msg = e.to_string();
            self.fail::<()>(&e);
            crate::feed::SourceError(msg)
        })
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
        self.cache.lock().map_err(|_| poisoned())?.put(key, Arc::clone(&b));
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
        let len = usize::from(u16::from_le_bytes([block.bytes[start], block.bytes[start + 1]]));
        let text = std::str::from_utf8(&block.bytes[start + 2..start + 2 + len])
            .map_err(|_| SegError::unreadable(name, 0, "string not utf-8"))?;
        Ok(Arc::from(text))
    }

    fn raw_ident(&self, id: u32) -> SegResult<(String, String)> {
        let s = self.string(1, id)?;
        let mut parts = s.split('\0');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(n), Some(e), None) => Ok((n.to_owned(), e.to_owned())),
            _ => Err(SegError::unreadable("strings", 0, "identity is not name NUL email")),
        }
    }

    fn seg_of(&self, rank: Rank) -> Option<(usize, u32)> {
        let i = self.segs.partition_point(|s| s.first <= rank.0).checked_sub(1)?;
        let s = &self.segs[i];
        (rank.0 - s.first < s.n).then_some((i, rank.0 - s.first))
    }

    /// Byte offset of ordinal `o` in segment `s`'s row payload, from a crc-checked block.
    fn row_offset(&self, s: usize, o: u32) -> SegResult<u64> {
        let seg = &self.segs[s];
        let bi = seg.idx_first.partition_point(|&f| f <= o) - 1;
        let start = seg.c.toc().partition_point(|e| e.section < section::ROW_INDEX);
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
        self.row_with(rank, None)
    }

    /// The commit at `rank` from section 1, checked against its record in `ROWS`, whose file
    /// rows go to `files` when given.
    fn row_with(&self, rank: Rank, files: Option<&mut Vec<FileRow>>) -> SegResult<CommitRow> {
        let (s, o) = self
            .seg_of(rank)
            .ok_or_else(|| SegError::unreadable("columns", 0, "rank out of range"))?;
        let seg = &self.segs[s];
        let bi = seg.col_first.partition_point(|&f| f <= o) - 1;
        let start = seg.c.toc().partition_point(|e| e.section < section::COMMITS);
        let b = self.block(Key::Seg(s, start + bi))?;
        let at = (o - seg.col_first[bi]) as usize * columns::ITEM;
        let bytes = b
            .bytes
            .get(at..at + columns::ITEM)
            .ok_or_else(|| SegError::unreadable(seg.c.name(), 0, "commit column short"))?;
        let it = columns::decode(&mut Cursor::new(bytes, seg.c.name(), at as u64))?;
        // A point lookup also reads the record: the two must agree, so a corrupt row block is
        // caught here too, not only by pages that read file rows.
        let rec = self.decode(rank, files)?;
        if !it.agrees(&rec) {
            return Err(SegError::unreadable(seg.c.name(), 0, "commit columns differ from rows"));
        }
        Ok(self.commit_row(rank, &it))
    }

    fn commit_row(&self, rank: Rank, it: &columns::Item) -> CommitRow {
        CommitRow {
            rank,
            ident: IdentId(it.ident),
            time: crate::plausible_time(it.time, self.latest),
            offset: it.offset,
            merge: it.merge,
            kind: it.kind.map(LabelId),
            scope: it.scope.map(LabelId),
            files: it.files,
            sums: it.sums,
        }
    }

    /// The rank of `oid`. Routing uses the open-time block boundaries, which were read without
    /// a crc, so neither answer trusts them alone: the block routed to must start with its
    /// boundary, and "absent" also needs the block after it to start with its own. A corrupt
    /// boundary is then `Unreadable`, never a commit reported missing.
    fn rank_of(&self, oid: &Oid) -> SegResult<Option<Rank>> {
        for (s, seg) in self.segs.iter().enumerate() {
            let start = seg.c.toc().partition_point(|e| e.section < section::OID_INDEX);
            let bi = seg.oid_first.partition_point(|f| f <= oid);
            let Some(bi) = bi.checked_sub(1) else {
                self.oid_block(s, start, 0)?;
                continue;
            };
            let b = self.oid_block(s, start, bi)?;
            let at = |i: usize| &b.bytes[i * OID_ENTRY..i * OID_ENTRY + 20];
            let (mut lo, mut hi) = (0, b.bytes.len() / OID_ENTRY);
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
            if bi + 1 < seg.oid_first.len() {
                self.oid_block(s, start, bi + 1)?;
            }
        }
        Ok(None)
    }

    /// `OID_INDEX` block `bi` of segment `s`, crc checked, and its first oid equal to the
    /// routing boundary open recorded for it.
    fn oid_block(&self, s: usize, start: usize, bi: usize) -> SegResult<Arc<Block>> {
        let seg = &self.segs[s];
        let b = self.block(Key::Seg(s, start + bi))?;
        let first = b.bytes.get(..20);
        if b.bytes.len() < OID_ENTRY || first != seg.oid_first.get(bi).map(<[u8; 20]>::as_slice) {
            return Err(SegError::unreadable(seg.c.name(), 0, "oid index routing"));
        }
        Ok(b)
    }

    /// Every file row with its rank, for the aggregates this source answers on its own (the
    /// segments answer them when this source is wrapped).
    fn each_row(&self, mut f: impl FnMut(&CommitRow, &[FileRow])) {
        let mut files = Vec::new();
        let mut s = scan::Scan::new(self, 0, self.len, true);
        while let Some(row) = s.next_row(&mut files) {
            f(&row, &files);
        }
    }
}

/// Writes a segment cache from records the way the build does.
///
/// For this crate's guards and the memory guard in `tests/disk_memory.rs`. Not an API.
#[cfg(any(test, feature = "oracle"))]
pub mod testing;

#[cfg(test)]
mod tests;
