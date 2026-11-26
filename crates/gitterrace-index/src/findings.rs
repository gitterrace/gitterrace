//! The review basket: an append-only log of findings in `.git/gitterrace/findings`.
//!
//! The file holds frames, one finding each: a fixed header (magic, payload length, payload
//! crc) and a payload of varint-framed fields read through the bounded [`Cursor`], so a length
//! that lies cannot size a buffer. A frame cut short by a crash is a torn tail: it is ignored
//! and the next write replaces the file without it. A damaged span (bad checksum, lying length,
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
/// Longest note kept, in bytes. The note is cut, not refused. Not configurable: part of the
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

/// `note` cut to [`MAX_NOTE`] bytes on a character boundary, control characters other than a
/// line break or tab dropped. The text is plain: nothing here is markup.
#[must_use]
pub fn cut_note(note: &str) -> String {
    let mut out = String::new();
    for c in note.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            continue;
        }
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
#[derive(Default)]
struct Scan {
    items: Vec<StoredFinding>,
    valid: usize,
    damaged: usize,
}

/// The payload of the whole, checksummed frame at `at`, and where it ends.
fn frame_at(bytes: &[u8], at: usize) -> Option<(usize, &[u8])> {
    let head = bytes.get(at..at + FIXED)?;
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let crc = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
    if &head[..4] != MAGIC || len > MAX_FRAME {
        return None;
    }
    let end = at + FIXED + len;
    let payload = bytes.get(at + FIXED..end)?;
    (crc32fast::hash(payload) == crc).then_some((end, payload))
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

/// The whole frames of `bytes`. A damaged span is skipped to the next whole frame and counted,
/// so one bad frame never hides the entries after it. Only a trailing incomplete frame is a
/// torn tail.
fn scan(bytes: &[u8]) -> io::Result<Scan> {
    let mut out = Scan::default();
    let mut at = 0;
    while at < bytes.len() {
        if let Some((end, payload)) = frame_at(bytes, at) {
            out.items.push(decode(payload)?);
            at = end;
            continue;
        }
        match (at + 1..bytes.len()).find(|&i| frame_at(bytes, i).is_some()) {
            Some(next) => {
                out.damaged += 1;
                at = next;
            }
            None if is_torn(bytes, at) => break,
            None => {
                out.damaged += 1;
                at = bytes.len();
            }
        }
    }
    out.valid = at;
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

fn encode_all(items: &[StoredFinding]) -> Vec<u8> {
    items.iter().flat_map(encode).collect()
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
        let Scan { mut items, valid, damaged } = scan;
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
        if damaged > 0 {
            return Err(refuse_rewrite(damaged));
        }
        items.push(f.clone());
        let over = items.len().saturating_sub(cap);
        items.drain(..over);
        cache::write_atomic(self.dir(), FILE, &encode_all(&items))?;
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
        if scan.damaged > 0 {
            return Err(refuse_rewrite(scan.damaged));
        }
        let mut items = scan.items;
        let before = items.len();
        if ids.is_empty() {
            items.clear();
        } else {
            items.retain(|x| !ids.contains(&x.id));
        }
        let gone = before - items.len();
        if gone > 0 {
            cache::write_atomic(self.dir(), FILE, &encode_all(&items))?;
        }
        Ok(gone)
    }
}

#[cfg(test)]
mod tests;
