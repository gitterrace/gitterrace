//! The message column: what commit search matches, one item per commit rank, so a search
//! reads the cache instead of walking git.
//!
//! Files `msg-<gen>-<n>.gts` (container kind 4, section 40), named in the manifest like every
//! other live file, so a crash, a cut or a flipped byte is the same unreadable cache as anywhere
//! else. In manifest order they cover the ranks of the order file from 0 without a gap: a fresh
//! column is one file, each later update adds one for its new ranks, and a segment merge writes
//! one file in the new canonical order. A column that does not cover every rank is not used.
//!
//! An item is the commit's committer time and parent ranks (to replay git's newest-first walk
//! exactly), its author as written and its full message as the walk decodes it. Items are
//! packed into blocks of about 64 KiB and compressed with zstd. An item over 512 KiB (a
//! giant generated message) is stored without its text and checked from git when searched.

use std::collections::{BinaryHeap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::seg::bounds::{Cursor, put_varint, unzigzag, zigzag};
use crate::seg::format::{Kind, StringBase, TocEntry, section};
use crate::seg::manifest::FileEntry;
use crate::seg::read::{Container, Store};
use crate::seg::write::ContainerWriter;
use crate::seg::{Oid, SegError, SegResult};

/// Raw bytes packed before a block is compressed. Not configurable, any of these: on-disk
/// format bounds that the writer and every reader must agree on.
const BLOCK_RAW: usize = 64 * 1024;
/// Largest item stored with its text.
const INLINE_MAX: usize = 512 * 1024;
/// Largest decompressed block a reader accepts: a full block plus one largest item.
const MAX_RAW: usize = 1 << 20;
/// Threads a scan decompresses on. Not configurable: like the index's parse threads, capped so
/// one search never takes every core of a large machine.
const MAX_THREADS: usize = 8;

const INLINE: u8 = 0;
const IN_GIT: u8 = 1;

/// One commit as the column stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Committer seconds, the key of the newest-first walk.
    pub time: i64,
    /// Ranks of the parents in the set, first parent first.
    pub parents: Vec<u32>,
    /// Author name and email as written, and the message. `None` for an item too large to
    /// keep, which search checks from git.
    pub text: Option<(Vec<u8>, Vec<u8>, String)>,
}

impl Item {
    /// The item of `t`, with parent ids mapped to ranks by `rank`. A parent outside the set
    /// (a shallow boundary) is left out: the walk cannot visit it either.
    pub fn of(t: gitterrace_git::CommitText, rank: &HashMap<Oid, u32>) -> Self {
        let parents = t
            .parents
            .iter()
            .filter_map(|p| crate::store::oid_of(p).and_then(|o| rank.get(&o).copied()))
            .collect();
        Self {
            time: t.committer_time,
            parents,
            text: Some((t.author_name, t.author_email, t.message)),
        }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        self.encode_as(out, self.text.as_ref());
        if out.len() - start > INLINE_MAX {
            out.truncate(start);
            self.encode_as(out, None);
        }
    }

    fn encode_as(&self, out: &mut Vec<u8>, text: Option<&(Vec<u8>, Vec<u8>, String)>) {
        out.push(if text.is_some() { INLINE } else { IN_GIT });
        put_varint(out, zigzag(self.time));
        put_varint(out, self.parents.len() as u64);
        for p in &self.parents {
            put_varint(out, u64::from(*p));
        }
        if let Some((name, email, message)) = text {
            for b in [name.as_slice(), email.as_slice(), message.as_bytes()] {
                put_varint(out, b.len() as u64);
                out.extend_from_slice(b);
            }
        }
    }

    fn decode(c: &mut Cursor<'_>, commits: u32) -> SegResult<Self> {
        let flag = c.u8()?;
        if flag != INLINE && flag != IN_GIT {
            return Err(c.err("message item flag"));
        }
        let time = unzigzag(c.varint()?);
        let n = c.count(MAX_RAW)?;
        let mut parents = Vec::new();
        for _ in 0..n {
            let p = u32::try_from(c.varint()?).map_err(|_| c.err("parent rank"))?;
            if p >= commits {
                return Err(c.err("parent rank past the set"));
            }
            parents.push(p);
        }
        let text = if flag == INLINE {
            let part = |c: &mut Cursor<'_>| -> SegResult<Vec<u8>> {
                let n = c.count(MAX_RAW)?;
                Ok(c.bytes(n)?.to_vec())
            };
            let name = part(c)?;
            let email = part(c)?;
            let message = String::from_utf8(part(c)?).map_err(|_| c.err("message not utf-8"))?;
            Some((name, email, message))
        } else {
            None
        };
        Ok(Self {
            time,
            parents,
            text,
        })
    }
}

/// Writes one message file. Drop without `finish` leaves only a `.tmp`, which the sweep
/// removes.
pub struct Writer {
    w: ContainerWriter,
    raw: Vec<u8>,
    items: u32,
}

impl Writer {
    pub fn new(dir: &Path, name: &str) -> SegResult<Self> {
        Ok(Self {
            w: ContainerWriter::new(dir, name, Kind::Messages, StringBase::default())?,
            raw: Vec::with_capacity(BLOCK_RAW * 2),
            items: 0,
        })
    }

    pub fn push(&mut self, item: &Item) -> SegResult<()> {
        item.encode(&mut self.raw);
        self.items += 1;
        if self.raw.len() >= BLOCK_RAW {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> SegResult<()> {
        if self.items == 0 {
            return Ok(());
        }
        let z = codec::pack(&self.raw)?;
        self.w.block(section::MESSAGES, self.items, &z)?;
        self.raw.clear();
        self.items = 0;
        Ok(())
    }

    pub fn finish(mut self) -> SegResult<FileEntry> {
        self.flush()?;
        self.w.finish()
    }
}

/// What a search hands the column.
pub struct MessageQuery<'a> {
    /// Whether the haystack `message \n name <email>` matches.
    pub is_match: &'a (dyn Fn(&str) -> bool + Sync),
    /// `name <email>` of an author as written, after the shown revision's mailmap.
    pub author: &'a (dyn Fn(&[u8], &[u8]) -> String + Sync),
    /// Whether the commit (hex id) matches, read from git: for items stored without text.
    pub in_git: &'a (dyn Fn(&str) -> std::io::Result<bool> + Sync),
}

/// Every rank's match result plus the graph the newest-first walk replays.
#[derive(Debug, Default)]
pub struct Scan {
    pub hits: Vec<bool>,
    pub time: Vec<i64>,
    /// Parents of rank `r`: `parents[offsets[r]..offsets[r + 1]]`.
    pub offsets: Vec<u32>,
    pub parents: Vec<u32>,
    /// Ranks whose item has no text, for the caller to check from git.
    pub in_git: Vec<u32>,
}

/// A store that answers commit search without walking git.
///
/// It gives every commit's match result and the graph to order them. The custom column is
/// one. Another backend implements the same call, so the two are measured on the same footing.
pub trait MessageIndex {
    fn scan(&self, query: &MessageQuery<'_>) -> SegResult<Scan>;
}

/// The commits a search lists, newest first as git's walk gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHits {
    pub ids: Vec<String>,
    /// More matched than `ids` holds.
    pub capped: bool,
}

/// The message files of a set that covers every rank, with the order file to name the hits.
pub struct MessageColumn {
    files: Vec<Container>,
    order: Container,
    commits: u32,
}

impl MessageColumn {
    /// The column of `store`, `None` when it does not cover every rank (off, or not built yet).
    /// A file that disagrees with its toc is unreadable.
    pub fn of(store: Store) -> SegResult<Option<Self>> {
        let commits = store.manifest.commits;
        let Store {
            messages, order, ..
        } = store;
        let Some(order) = order else {
            return Ok(None);
        };
        let mut total = 0u64;
        for f in &messages {
            let mut items = 0u64;
            for e in f.toc() {
                if e.section != section::MESSAGES {
                    return Err(SegError::unreadable(f.name(), e.offset, "foreign section"));
                }
                items += u64::from(e.items);
            }
            if items != u64::from(f.items()) {
                return Err(SegError::unreadable(
                    f.name(),
                    0,
                    "item count differs from toc",
                ));
            }
            total += items;
        }
        if total > u64::from(commits) {
            return Err(SegError::unreadable(
                "MANIFEST",
                0,
                "message column longer than the set",
            ));
        }
        if messages.is_empty() || total < u64::from(commits) {
            return Ok(None);
        }
        Ok(Some(Self {
            files: messages,
            order,
            commits,
        }))
    }

    pub const fn commits(&self) -> u32 {
        self.commits
    }

    /// The oid at `rank`, through a crc-checked read of its order block.
    fn oid(&self, rank: u32, cache: &mut Option<(u32, Vec<u8>)>) -> SegResult<Oid> {
        let mut first = 0u32;
        for e in self.order.blocks(section::ORDER) {
            if rank < first.saturating_add(e.items) {
                if cache.as_ref().is_none_or(|(f, _)| *f != first) {
                    let mut buf = Vec::new();
                    self.order.read_block(e, &mut buf)?;
                    *cache = Some((first, buf));
                }
                let bytes = cache
                    .as_ref()
                    .map(|(_, b)| b.as_slice())
                    .unwrap_or_default();
                let at = (rank - first) as usize * 20;
                let mut c = Cursor::new(
                    bytes.get(at..).unwrap_or_default(),
                    self.order.name(),
                    e.offset,
                );
                return c.array::<20>();
            }
            first = first.saturating_add(e.items);
        }
        Err(SegError::unreadable(
            self.order.name(),
            0,
            "rank past the order",
        ))
    }

    /// The commits matching `query`, newest first as `git log` (and the walk commit search ran
    /// before) lists them, at most `most`. `tip` is the commit searched from, which must be the
    /// set's newest rank, else the column is not current and this says so with `None`.
    pub fn search(
        &self,
        tip: &str,
        query: &MessageQuery<'_>,
        most: usize,
    ) -> SegResult<Option<MessageHits>> {
        let mut cache = None;
        let Some(last) = self.commits.checked_sub(1) else {
            return Ok(None);
        };
        if crate::store::oid_of(tip) != Some(self.oid(last, &mut cache)?) {
            return Ok(None);
        }
        let mut scan = self.scan(query)?;
        for &r in &scan.in_git {
            let id = hex(&self.oid(r, &mut cache)?);
            scan.hits[r as usize] = (query.in_git)(&id).map_err(SegError::Io)?;
        }
        let total = scan.hits.iter().filter(|h| **h).count();
        let mut ids = Vec::new();
        if total > 0 {
            for r in walk(&scan, last) {
                if scan.hits[r as usize] {
                    if ids.len() >= most {
                        break;
                    }
                    ids.push(hex(&self.oid(r, &mut cache)?));
                }
            }
        }
        Ok(Some(MessageHits {
            capped: total > ids.len(),
            ids,
        }))
    }

    /// Every item in rank order, for a merge that rewrites the column.
    pub(crate) fn items(&self) -> SegResult<Vec<Item>> {
        let mut out = Vec::with_capacity(self.commits as usize);
        let mut z = codec::Unpacker::new()?;
        let mut buf = Vec::new();
        for f in &self.files {
            for e in f.toc() {
                let raw = block(f, e, &mut z, &mut buf)?;
                let mut c = Cursor::new(&raw, f.name(), e.offset);
                for _ in 0..e.items {
                    out.push(Item::decode(&mut c, self.commits)?);
                }
                if !c.is_empty() {
                    return Err(c.err("bytes after the last item"));
                }
            }
        }
        Ok(out)
    }
}

impl MessageIndex for MessageColumn {
    fn scan(&self, query: &MessageQuery<'_>) -> SegResult<Scan> {
        let mut blocks: Vec<(&Container, &TocEntry, u32)> = Vec::new();
        let mut first = 0u32;
        for f in &self.files {
            for e in f.toc() {
                blocks.push((f, e, first));
                first += e.items;
            }
        }
        let threads = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .clamp(1, MAX_THREADS)
            .min(blocks.len().max(1));
        let next = AtomicUsize::new(0);
        let commits = self.commits;
        let results: Vec<SegResult<Vec<(usize, Part)>>> = std::thread::scope(|s| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    s.spawn(|| -> SegResult<Vec<(usize, Part)>> {
                        let mut z = codec::Unpacker::new()?;
                        let mut buf = Vec::new();
                        let mut names: HashMap<(Vec<u8>, Vec<u8>), String> = HashMap::new();
                        let mut done = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(&(f, e, first)) = blocks.get(i) else {
                                return Ok(done);
                            };
                            let raw = block(f, e, &mut z, &mut buf)?;
                            let part = scan_block(&raw, f, e, first, commits, query, &mut names)?;
                            done.push((i, part));
                        }
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|w| {
                    w.join().unwrap_or_else(|_| {
                        Err(SegError::Io(std::io::Error::other("message scan panicked")))
                    })
                })
                .collect()
        });
        let mut parts: Vec<Option<Part>> = (0..blocks.len()).map(|_| None).collect();
        for r in results {
            for (i, p) in r? {
                parts[i] = Some(p);
            }
        }
        let n = self.commits as usize;
        let mut out = Scan {
            hits: Vec::with_capacity(n),
            time: Vec::with_capacity(n),
            offsets: Vec::with_capacity(n + 1),
            parents: Vec::new(),
            in_git: Vec::new(),
        };
        out.offsets.push(0);
        for p in parts.into_iter().flatten() {
            out.hits.extend(p.hits);
            out.time.extend(p.time);
            let base = u32::try_from(out.parents.len()).unwrap_or(u32::MAX);
            out.offsets.extend(p.ends.iter().map(|e| base + e));
            out.parents.extend(p.parents);
            out.in_git.extend(p.in_git);
        }
        if out.hits.len() != n {
            return Err(SegError::unreadable("MANIFEST", 0, "message column short"));
        }
        Ok(out)
    }
}

/// One block's share of a [`Scan`].
struct Part {
    hits: Vec<bool>,
    time: Vec<i64>,
    /// End of each item's parents in `parents`.
    ends: Vec<u32>,
    parents: Vec<u32>,
    in_git: Vec<u32>,
}

fn block(
    f: &Container,
    e: &TocEntry,
    z: &mut codec::Unpacker,
    buf: &mut Vec<u8>,
) -> SegResult<Vec<u8>> {
    let packed = f.read_block(e, buf)?;
    z.unpack(packed, MAX_RAW)
        .ok_or_else(|| SegError::unreadable(f.name(), e.offset, "message block does not inflate"))
}

fn scan_block(
    raw: &[u8],
    file: &Container,
    entry: &TocEntry,
    first: u32,
    commits: u32,
    query: &MessageQuery<'_>,
    names: &mut HashMap<(Vec<u8>, Vec<u8>), String>,
) -> SegResult<Part> {
    let n = entry.items as usize;
    let mut part = Part {
        hits: Vec::with_capacity(n),
        time: Vec::with_capacity(n),
        ends: Vec::with_capacity(n),
        parents: Vec::new(),
        in_git: Vec::new(),
    };
    let mut cur = Cursor::new(raw, file.name(), entry.offset);
    let mut hay = String::new();
    for i in 0..entry.items {
        let item = Item::decode(&mut cur, commits)?;
        part.time.push(item.time);
        part.parents.extend(&item.parents);
        part.ends
            .push(u32::try_from(part.parents.len()).unwrap_or(u32::MAX));
        let hit = if let Some((name, email, message)) = item.text {
            let key = (name, email);
            if !names.contains_key(&key) {
                let shown = (query.author)(&key.0, &key.1);
                names.insert(key.clone(), shown);
            }
            hay.clear();
            hay.push_str(&message);
            hay.push('\n');
            hay.push_str(names.get(&key).map_or("", String::as_str));
            (query.is_match)(&hay)
        } else {
            part.in_git.push(first + i);
            false
        };
        part.hits.push(hit);
    }
    if !cur.is_empty() {
        return Err(cur.err("bytes after the last item"));
    }
    Ok(part)
}

/// A walk queue entry ordered by its key alone, as git's commit-date queue orders them: the
/// same heap fed the same pushes pops in the same order, ties included.
struct Queued {
    time: i64,
    rank: u32,
}

impl PartialEq for Queued {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time
    }
}
impl Eq for Queued {}
impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Queued {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.time.cmp(&other.time)
    }
}

/// Ranks in the order of git's newest-first walk from `tip`: pop the newest queued commit,
/// queue each parent not seen yet in parent order.
fn walk(scan: &Scan, tip: u32) -> impl Iterator<Item = u32> + '_ {
    let mut seen = vec![false; scan.time.len()];
    let mut heap = BinaryHeap::new();
    if let Some(s) = seen.get_mut(tip as usize) {
        *s = true;
        heap.push(Queued {
            time: scan.time[tip as usize],
            rank: tip,
        });
    }
    std::iter::from_fn(move || {
        let q = heap.pop()?;
        let r = q.rank as usize;
        let (a, b) = (scan.offsets[r] as usize, scan.offsets[r + 1] as usize);
        for &p in scan.parents.get(a..b).unwrap_or_default() {
            if let Some(s) = seen.get_mut(p as usize)
                && !*s
            {
                *s = true;
                heap.push(Queued {
                    time: scan.time[p as usize],
                    rank: p,
                });
            }
        }
        Some(q.rank)
    })
}

pub(crate) fn hex(oid: &Oid) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(40);
    for b in oid {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    s
}

mod build;
mod codec;

pub(crate) use build::{extend, remap};

#[cfg(test)]
mod tests;
