//! The review basket: an append-only log of findings in `.git/gitterrace/findings`.
//!
//! The file holds frames, one finding each: a fixed header (magic, payload length, payload
//! crc) and a payload of varint-framed fields read through the bounded [`Cursor`], so a length
//! that lies cannot size a buffer. A frame cut short by a crash is a torn tail: it is ignored
//! and the next write replaces the file without it, unless the tail already holds a whole
//! frame (a length field bumped past the file looks exactly like a cut write): then it is a
//! damaged span, kept. A damaged span (bad checksum, lying length,
//! unknown magic) is skipped to the next whole frame and reported, and no write rewrites a file
//! holding one: an add appends, a remove or a cap trim is refused, so unreadable bytes are kept.
//! A frame whose checksum holds but whose
//! payload does not decode comes from a build this one does not know: reading it is an error,
//! never a guess and never a deletion.
//!
//! Not rebuildable: this is the user's data. It is only ever written through `cache`, so it
//! cannot leave `.git/gitterrace/`.

use std::io::{self, Write};

use gitterrace_git::content_id;

use crate::seg::bounds::{Cursor, MAX_STRING, put_varint, unzigzag, zigzag};
use crate::{Index, cache};

pub(crate) const FILE: &str = "findings";

const MAGIC: &[u8; 4] = b"GTF1";
const FIXED: usize = 12;
/// The one op a frame holds today: add a finding. Anything else is a newer build's.
const OP_ADD: u8 = 1;
/// A payload bigger than this is no frame of ours. Not configurable: a bound on a file the
/// repository may have planted.
const MAX_FRAME: usize = 256 << 10;
/// Longest note kept, in UTF-8 bytes. The web write refuses a longer note and names this limit.
/// Not configurable: part of the
/// finding shape the API documents.
pub const MAX_NOTE: usize = 2000;

/// One entry of the basket as it is stored: what the user chose, with nothing computed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFinding {
    pub id: String,
    pub kind: String,
    pub rev: String,
    pub sha: String,
    pub path: Option<String>,
    pub lines: Option<(u32, u32)>,
    pub query: Option<String>,
    pub note: String,
    pub added: i64,
}

impl StoredFinding {
    /// `self` with its note cut and its id set: `f-` and 8 hex of the git blob id of every
    /// other field, so the same selection added in the same second is the same finding.
    #[must_use]
    pub fn sealed(mut self) -> Self {
        self.note = cut_note(&self.note);
        let l = self.lines.map_or_else(String::new, |(a, b)| format!("{a}-{b}"));
        let key = [
            self.kind.as_str(),
            &self.sha,
            self.path.as_deref().unwrap_or(""),
            &l,
            self.query.as_deref().unwrap_or(""),
            &self.note,
            &self.added.to_string(),
        ];
        let id = content_id(key.join("\0").as_bytes());
        self.id = format!("f-{}", &id[..8]);
        self
    }
}

/// `note` without control characters other than a line break or tab. The text is plain:
/// nothing here is markup.
#[must_use]
pub fn clean_note(note: &str) -> String {
    note.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

/// `note` cleaned by [`clean_note`] and cut to [`MAX_NOTE`] bytes on a character boundary.
///
/// The web write refuses a note that is over the limit once cleaned, so this cut only guards the
/// store against any other caller: a user is never told "added" for a cut note.
#[must_use]
pub fn cut_note(note: &str) -> String {
    let mut out = String::new();
    for c in clean_note(note).chars() {
        if out.len() + c.len_utf8() > MAX_NOTE {
            break;
        }
        out.push(c);
    }
    out
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn put_opt(out: &mut Vec<u8>, s: Option<&str>) {
    match s {
        Some(s) => {
            out.push(1);
            put_str(out, s);
        }
        None => out.push(0),
    }
}

/// One frame for `f`.
fn encode(f: &StoredFinding) -> Vec<u8> {
    let mut p = vec![OP_ADD];
    for s in [&f.id, &f.kind, &f.rev, &f.sha] {
        put_str(&mut p, s);
    }
    put_opt(&mut p, f.path.as_deref());
    match f.lines {
        Some((a, b)) => {
            p.push(1);
            put_varint(&mut p, u64::from(a));
            put_varint(&mut p, u64::from(b));
        }
        None => p.push(0),
    }
    put_opt(&mut p, f.query.as_deref());
    put_str(&mut p, &f.note);
    put_varint(&mut p, zigzag(f.added));
    let mut out = Vec::with_capacity(FIXED + p.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(p.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(&p).to_le_bytes());
    out.extend_from_slice(&p);
    out
}

fn bad(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

fn string(c: &mut Cursor<'_>) -> io::Result<String> {
    let n = c.count(MAX_STRING).map_err(bad)?;
    String::from_utf8(c.bytes(n).map_err(bad)?.to_vec()).map_err(bad)
}

fn opt(c: &mut Cursor<'_>) -> io::Result<Option<String>> {
    match c.u8().map_err(bad)? {
        0 => Ok(None),
        1 => string(c).map(Some),
        _ => Err(bad("flag is neither 0 nor 1")),
    }
}

fn line(c: &mut Cursor<'_>) -> io::Result<u32> {
    u32::try_from(c.varint().map_err(bad)?).map_err(bad)
}

/// The finding in one frame's payload. Every length is checked against a limit before it sizes
/// anything, so a payload that lies about one is an error.
fn decode(payload: &[u8]) -> io::Result<StoredFinding> {
    let mut c = Cursor::new(payload, FILE, 0);
    if c.u8().map_err(bad)? != OP_ADD {
        return Err(bad("an op this build does not know: the file belongs to a newer gitterrace"));
    }
    let (id, kind, rev, sha) = (string(&mut c)?, string(&mut c)?, string(&mut c)?, string(&mut c)?);
    let path = opt(&mut c)?;
    let lines = match c.u8().map_err(bad)? {
        0 => None,
        1 => Some((line(&mut c)?, line(&mut c)?)),
        _ => return Err(bad("flag is neither 0 nor 1")),
    };
    let query = opt(&mut c)?;
    let note = string(&mut c)?;
    let added = unzigzag(c.varint().map_err(bad)?);
    if !c.is_empty() {
        return Err(bad("bytes after the last field"));
    }
    Ok(StoredFinding { id, kind, rev, sha, path, lines, query, note, added })
}

/// What a pass over the file found. `valid` is where reading stopped cleanly: the file's length
/// unless an incomplete last frame (a torn tail) follows. `damaged` counts spans that are no
/// frame and no torn tail: a flipped byte, a lying length, a magic this build does not know.
/// `incomplete` is set when the checksum budget ran out before the file was read through: the
/// rest is unread, so no write may rewrite or extend the file. `hashed` is the checksum input
/// the pass spent.
#[derive(Default)]
struct Scan {
    items: Vec<StoredFinding>,
    valid: usize,
    damaged: usize,
    incomplete: bool,
    hashed: usize,
}

/// The checksum work a pass may spend: twice the file, so the whole frames (at most the file in
/// all) never starve the search for the next one, and a planted run of plausible headers cannot
/// make the cost grow with their number. Not configurable: a bound on a file the repository may
/// have planted.
const fn hash_budget(len: usize) -> usize {
    len.saturating_mul(2)
}

/// The payload of the whole, checksummed frame at `at`, and where it ends. `Err` when hashing it
/// would pass `budget`, the checksum input still allowed.
fn frame_at<'a>(
    bytes: &'a [u8],
    at: usize,
    budget: &mut usize,
) -> Result<Option<(usize, &'a [u8])>, ()> {
    let Some(head) = bytes.get(at..at + FIXED) else {
        return Ok(None);
    };
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let crc = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
    if &head[..4] != MAGIC || len > MAX_FRAME {
        return Ok(None);
    }
    let end = at + FIXED + len;
    let Some(payload) = bytes.get(at + FIXED..end) else {
        return Ok(None);
    };
    *budget = budget.checked_sub(len).ok_or(())?;
    Ok((crc32fast::hash(payload) == crc).then_some((end, payload)))
}

/// Whether the bytes after the header at `at` hold a whole frame: some prefix of them has the
/// header's checksum. A cut write never does, but a length field bumped past the file's end looks
/// like one, so such a tail is never discarded. Only called for a header whose length is within
/// [`MAX_FRAME`], so it hashes less than that, once per pass.
fn holds_whole_frame(bytes: &[u8], at: usize) -> bool {
    let Some(head) = bytes.get(at..at + FIXED) else {
        return false;
    };
    let crc = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
    let mut hasher = crc32fast::Hasher::new();
    bytes[at + FIXED..].iter().any(|b| {
        hasher.update(&[*b]);
        hasher.clone().finalize() == crc
    })
}

/// Whether the bytes from `at` are the start of a frame the file ends before finishing.
fn is_torn(bytes: &[u8], at: usize) -> bool {
    let rest = &bytes[at..];
    if rest.len() < FIXED {
        return MAGIC.starts_with(&rest[..rest.len().min(4)]);
    }
    let len = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
    &rest[..4] == MAGIC && len <= MAX_FRAME && FIXED + len > rest.len()
}

/// The next whole frame start after `from`. `Err` when the budget ran out first.
fn resync(bytes: &[u8], from: usize, budget: &mut usize) -> Result<Option<usize>, ()> {
    for i in from..bytes.len() {
        if frame_at(bytes, i, budget)?.is_some() {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// The whole frames of `bytes`. A damaged span is skipped to the next whole frame and counted,
/// so one bad frame never hides the entries after it. Only a trailing incomplete frame is a
/// torn tail. The checksum work is bounded by [`hash_budget`]: past it the scan stops, counts
/// the unread rest as damaged and reports `incomplete`.
fn scan(bytes: &[u8]) -> io::Result<Scan> {
    let mut out = Scan::default();
    let total = hash_budget(bytes.len());
    let mut budget = total;
    let mut at = 0;
    while at < bytes.len() {
        match frame_at(bytes, at, &mut budget) {
            Ok(Some((end, payload))) => {
                out.items.push(decode(payload)?);
                at = end;
                continue;
            }
            Ok(None) => {}
            Err(()) => {
                out.incomplete = true;
                break;
            }
        }
        match resync(bytes, at + 1, &mut budget) {
            Ok(Some(next)) => {
                out.damaged += 1;
                at = next;
            }
            Ok(None) if is_torn(bytes, at) && !holds_whole_frame(bytes, at) => break,
            Ok(None) => {
                out.damaged += 1;
                at = bytes.len();
            }
            Err(()) => {
                out.incomplete = true;
                break;
            }
        }
    }
    if out.incomplete {
        out.damaged += 1;
    }
    out.valid = at;
    out.hashed = total - budget;
    Ok(out)
}

fn read(idx: &Index) -> io::Result<Option<(Scan, usize)>> {
    let Some(bytes) = cache::read(idx.dir(), FILE)? else {
        return Ok(None);
    };
    Ok(Some((scan(&bytes)?, bytes.len())))
}

/// The error of a write that would rewrite the file and so destroy bytes it cannot read.
fn refuse_rewrite(damaged: usize) -> io::Error {
    bad(format_args!(
        "the review basket holds {damaged} damaged span(s) this build cannot read: not rewriting \
         it, so nothing is lost. Keep a copy of .git/gitterrace/{FILE} and remove it to start over"
    ))
}

/// The one way the file is rewritten, so the tail rule holds for every caller: a pass that found
/// damaged or unread bytes (`damaged`, which an incomplete scan always counts) is refused, so a
/// rewrite drops nothing but a plain torn tail.
fn rewrite(idx: &Index, damaged: usize, items: &[StoredFinding]) -> io::Result<()> {
    if damaged > 0 {
        return Err(refuse_rewrite(damaged));
    }
    let bytes: Vec<u8> = items.iter().flat_map(encode).collect();
    cache::write_atomic(idx.dir(), FILE, &bytes)
}

impl Index {
    /// The basket, oldest first. A missing file is an empty basket.
    ///
    /// # Errors
    /// When the file cannot be read, or holds a frame this build cannot decode.
    pub fn findings_list(&self) -> io::Result<Vec<StoredFinding>> {
        let Some((scan, _)) = read(self)? else {
            return Ok(Vec::new());
        };
        if scan.damaged > 0 {
            crate::warn(format_args!(
                "{FILE} holds {} damaged span(s): skipped, the readable entries are listed",
                scan.damaged
            ));
        }
        Ok(scan.items)
    }

    /// Adds `f`, dropping the oldest entries past `cap_entries`. Returns false when a finding
    /// with the same id is already there.
    ///
    /// # Errors
    /// When the cache cannot be written: a newer build owns it, or the disk refuses.
    pub fn findings_add(&self, f: &StoredFinding, cap_entries: usize) -> io::Result<bool> {
        self.writable()?;
        // The cache's writer lock: a rewrite replaces the file, so it must not run between
        // another writer's scan and append.
        let _lock = crate::store::WriteLock::take(self.dir())?;
        let (scan, size) = read(self)?.unwrap_or_default();
        let Scan { mut items, valid, damaged, incomplete, .. } = scan;
        if incomplete {
            // Appending would bury the new entry behind bytes never read.
            return Err(refuse_rewrite(damaged));
        }
        if items.iter().any(|x| x.id == f.id) {
            return Ok(false);
        }
        let cap = cap_entries.max(1);
        if valid == size && items.len() < cap {
            let mut out = cache::open_append(self.dir(), FILE)?;
            out.write_all(&encode(f))?;
            out.sync_data()?;
            return Ok(true);
        }
        items.push(f.clone());
        let over = items.len().saturating_sub(cap);
        items.drain(..over);
        rewrite(self, damaged, &items)?;
        Ok(true)
    }

    /// Removes the findings named in `ids`, or all of them when `ids` is empty. Returns how
    /// many went.
    ///
    /// # Errors
    /// When the cache cannot be written.
    pub fn findings_remove(&self, ids: &[String]) -> io::Result<usize> {
        self.writable()?;
        let _lock = crate::store::WriteLock::take(self.dir())?;
        let Some((scan, _)) = read(self)? else {
            return Ok(0);
        };
        let damaged = scan.damaged;
        let mut items = scan.items;
        let before = items.len();
        if ids.is_empty() {
            items.clear();
        } else {
            items.retain(|x| !ids.contains(&x.id));
        }
        let gone = before - items.len();
        if gone > 0 || damaged > 0 {
            rewrite(self, damaged, &items)?;
        }
        Ok(gone)
    }
}

#[cfg(test)]
mod tests;
