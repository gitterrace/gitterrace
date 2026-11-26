//! Checkpoints of uncommitted work: an append-only log in `.git/gitterrace/checkpoints`.
//!
//! The one file holds frames. A frame is a fixed header (magic, header length, header crc, blob
//! length), a JSON header naming the checkpoint and its files, and the zstd copies of the changed
//! text files. A list reads only the headers and skips the blobs. A frame cut short by a crash is
//! a torn tail: it is ignored, and the next write replaces the file without it.
//!
//! Not rebuildable: this is the user's data. It lives beside the index and, like it, is only ever
//! written through `cache`, so it cannot leave `.git/gitterrace/`.

use std::io::{self, Read, Seek, SeekFrom};

use gitterrace_git::{SnapFile, Snapshot, content_id};
use serde::{Deserialize, Serialize};

use crate::{Index, cache};

pub(crate) const FILE: &str = "checkpoints";

const MAGIC: &[u8; 4] = b"GTC1";
const FIXED: u64 = 20;
/// A header bigger than this is no header of ours. Not configurable: a bound on a file the
/// repository may have planted.
const MAX_HEADER: u32 = 16 << 20;
/// Checkpoints kept whatever the byte cap says: each holds a header of path names.
const MAX_CHECKPOINTS: usize = 200;
/// Longest label a checkpoint keeps, in characters. The one limit: callers pass labels as typed.
/// Not configurable: a label is a short note, not content.
const MAX_LABEL: usize = 200;
/// Largest copy, packed or unpacked: `review.checkpoint_file_kb` tops out at 4 MiB, so a larger
/// claim is a planted size.
const MAX_COPY: u64 = 4 << 20;
/// Largest packed copy: what zstd can make of an incompressible `MAX_COPY` (its worst-case bound
/// is the input plus 1/256th plus a few bytes of framing). The writer and the reader share it, so
/// a copy the writer packs is never refused as too long.
const MAX_PACKED: u64 = MAX_COPY + (MAX_COPY >> 8) + 128;
/// Most bytes of copies one read unpacks, so many small claims cannot add up to gigabytes. The
/// writer keeps no more than this unpacked per checkpoint, so a restore never drops a copy the
/// log holds.
const MAX_RESTORE: u64 = 256 << 20;
/// Largest blob of one frame: `review.checkpoint_mb` tops out at 1 GiB.
const MAX_BLOB: u64 = 1 << 30;
/// Most bytes of headers one log holds, so a planted log cannot make every list parse
/// gigabytes of JSON.
const MAX_HEADERS: u64 = 64 << 20;
const ZSTD_LEVEL: i32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Copy {
    off: u64,
    len: u64,
    size: u64,
    crc: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileMeta {
    path: String,
    hash: String,
    copy: Option<Copy>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Meta {
    id: String,
    created: i64,
    head: String,
    auto: bool,
    label: String,
    files: Vec<FileMeta>,
    /// The scan that listed `files` stopped at `limits.worktree_files`. Absent in a log from
    /// before the field, which reads as complete.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    omitted: bool,
}

/// What a list shows of one checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointInfo {
    pub id: String,
    pub created: i64,
    pub head: String,
    pub files: usize,
    /// Bytes of copies still held.
    pub bytes: u64,
    pub auto: bool,
    pub label: String,
    /// The working tree had more changed files than the scan lists, so the checkpoint may not
    /// hold them all.
    pub files_omitted: bool,
}

/// The checkpoints, newest first, and what they hold in all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckpointList {
    pub items: Vec<CheckpointInfo>,
    pub bytes_used: u64,
}

/// One checkpoint read back: where HEAD was and what the working tree held that differed from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub head: String,
    /// See [`CheckpointInfo::files_omitted`].
    pub files_omitted: bool,
    pub files: Vec<SnapFile>,
}

/// A frame found in the log and where its blobs start.
struct Frame {
    meta: Meta,
    blob_at: u64,
    /// Bytes of the header as written, which the reader counts against `MAX_HEADERS`.
    hdr_len: u32,
}

/// The id of a state: `cp-` and 12 hex of the git blob id of its HEAD and manifest (paths and
/// content hashes). The same files at another HEAD are another state: the files a checkpoint
/// does not list read as HEAD then.
fn id_of(files: &[SnapFile], head: &str) -> String {
    let mut rows: Vec<String> = files.iter().map(|f| format!("{}\0{}\n", f.path, f.hash)).collect();
    rows.sort();
    let id = content_id(format!("{head}\n{}", rows.concat()).as_bytes());
    format!("cp-{}", &id[..12])
}

const fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

const fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7]])
}

/// What a scan of the log found.
struct Scan {
    frames: Vec<Frame>,
    /// Where the last whole frame ends. Anything after that is a torn tail or damage.
    valid: u64,
    /// The bytes after `valid` are not a frame cut short but data this build cannot read: a bad
    /// magic, a size past a bound, a header that fails its checksum or parse, or too many frames.
    damaged: bool,
}

/// Keeps the copies of `meta` that lie inside a blob of `blob_len` bytes and are within bounds.
/// A frame we wrote always passes. A planted one loses the copy and so reads as gone, instead of
/// feeding an offset or a size it made up to a read.
fn sound(meta: &mut Meta, blob_len: u64) {
    let mut used = 0u64;
    for f in &mut meta.files {
        let Some(c) = &f.copy else { continue };
        let fits = c.len <= MAX_PACKED
            && c.size <= MAX_COPY
            && c.off.checked_add(c.len).is_some_and(|e| e <= blob_len)
            && used.checked_add(c.len).is_some_and(|u| u <= blob_len);
        if fits {
            used += c.len;
        } else {
            f.copy = None;
        }
    }
}

/// The whole frames of the log and where the last one ends.
fn scan(f: &mut std::fs::File) -> io::Result<Scan> {
    let len = f.metadata()?.len();
    let mut out = Scan { frames: Vec::new(), valid: 0, damaged: false };
    let (mut at, mut headers, mut blobs) = (0u64, 0u64, 0u64);
    while at.saturating_add(FIXED) <= len {
        f.seek(SeekFrom::Start(at))?;
        let mut fixed = [0u8; FIXED as usize];
        f.read_exact(&mut fixed)?;
        let (hdr_len, hdr_crc, blob_len) =
            (u32_at(&fixed, 4), u32_at(&fixed, 8), u64_at(&fixed, 12));
        headers += u64::from(hdr_len);
        blobs = blobs.saturating_add(blob_len);
        if &fixed[..4] != MAGIC
            || hdr_len > MAX_HEADER
            || headers > MAX_HEADERS
            || blob_len > MAX_BLOB
            || blobs > MAX_BLOB.saturating_mul(2)
            || out.frames.len() >= MAX_CHECKPOINTS
        {
            out.damaged = true;
            break;
        }
        let Some(end) = (at + FIXED).checked_add(u64::from(hdr_len) + blob_len) else {
            out.damaged = true;
            break;
        };
        if end > len {
            break;
        }
        let mut hdr = vec![0; hdr_len as usize];
        f.read_exact(&mut hdr)?;
        let Ok(mut meta) = serde_json::from_slice::<Meta>(&hdr)
            .map_err(drop)
            .and_then(|m| (crc32fast::hash(&hdr) == hdr_crc).then_some(m).ok_or(()))
        else {
            out.damaged = true;
            break;
        };
        sound(&mut meta, blob_len);
        out.frames.push(Frame { meta, blob_at: at + FIXED + u64::from(hdr_len), hdr_len });
        at = end;
    }
    out.valid = at;
    Ok(out)
}

/// The packed bytes of one copy, read by its own offset and length, `None` when they fail the
/// copy's checksum. `sound` has already bounded both.
fn packed(f: &mut std::fs::File, frame: &Frame, c: &Copy) -> io::Result<Option<Vec<u8>>> {
    f.seek(SeekFrom::Start(frame.blob_at + c.off))?;
    let mut z = vec![0; usize::try_from(c.len).map_err(io::Error::other)?];
    f.read_exact(&mut z)?;
    Ok((crc32fast::hash(&z) == c.crc).then_some(z))
}

/// The frame as it will be written again: its sound copies packed one after another, so bytes
/// the log held but no copy names are not carried over.
fn repack(f: &mut std::fs::File, frame: &Frame) -> io::Result<(Meta, Vec<u8>)> {
    let mut meta = frame.meta.clone();
    let mut blobs = Vec::new();
    for m in &mut meta.files {
        let Some(c) = m.copy.as_mut() else { continue };
        match packed(f, frame, c)? {
            Some(z) => {
                c.off = blobs.len() as u64;
                blobs.extend_from_slice(&z);
            }
            None => m.copy = None,
        }
    }
    Ok((meta, blobs))
}

fn encode(meta: &Meta, blobs: &[u8]) -> io::Result<Vec<u8>> {
    let hdr = serde_json::to_vec(meta).map_err(io::Error::other)?;
    // The reader calls a longer header damage and then refuses every later write, so a header it
    // would refuse is refused here, before anything reaches the log.
    if u32::try_from(hdr.len()).map_or(true, |n| n > MAX_HEADER) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the checkpoint lists too many or too long paths to store; \
             lower review.worktree_files",
        ));
    }
    let mut out = Vec::with_capacity(FIXED as usize + hdr.len() + blobs.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&u32::try_from(hdr.len()).map_err(io::Error::other)?.to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(&hdr).to_le_bytes());
    out.extend_from_slice(&(blobs.len() as u64).to_le_bytes());
    out.extend_from_slice(&hdr);
    out.extend_from_slice(blobs);
    Ok(out)
}

fn held(meta: &Meta) -> u64 {
    meta.files.iter().filter_map(|f| f.copy.as_ref()).fold(0, |n, c| n.saturating_add(c.len))
}

/// How many of the oldest frames, with headers of `sizes` bytes, go so the rest are within
/// `MAX_HEADERS`. The newest never goes.
fn oldest_to_drop(sizes: &[u64]) -> usize {
    let mut headers: u64 = sizes.iter().sum();
    let mut drop = 0;
    while headers > MAX_HEADERS && drop + 1 < sizes.len() {
        headers -= sizes[drop];
        drop += 1;
    }
    drop
}

/// The header length of an encoded frame.
fn hdr_of(frame: &[u8]) -> u64 {
    u64::from(u32_at(frame, 4))
}

/// A checkpoint of `files` as one frame. Compression stops once the copies packed so far pass
/// `cap`: the frame could not be kept whole then, so peak memory is the cap and one file, not
/// every file of the snapshot. The files after that keep their names and hashes.
fn build(
    snap: &Snapshot,
    head: &str,
    auto: bool,
    label: &str,
    created: i64,
    cap: u64,
) -> io::Result<(Meta, Vec<u8>)> {
    let files = &snap.files;
    let mut blobs = Vec::new();
    let mut unpacked = 0u64;
    let mut metas = Vec::with_capacity(files.len());
    for f in files {
        let copy = match &f.text {
            Some(t)
                if (blobs.len() as u64) <= cap
                    && t.len() as u64 <= MAX_COPY
                    && unpacked + t.len() as u64 <= MAX_RESTORE =>
            {
                let z = zstd::bulk::compress(t, ZSTD_LEVEL)?;
                unpacked += t.len() as u64;
                let c = Copy {
                    off: blobs.len() as u64,
                    len: z.len() as u64,
                    size: t.len() as u64,
                    crc: crc32fast::hash(&z),
                };
                blobs.extend_from_slice(&z);
                Some(c)
            }
            _ => None,
        };
        metas.push(FileMeta { path: f.path.clone(), hash: f.hash.clone(), copy });
    }
    let meta = Meta {
        id: id_of(files, head),
        created,
        head: head.to_owned(),
        auto,
        label: label.chars().take(MAX_LABEL).collect(),
        files: metas,
        omitted: snap.files_omitted,
    };
    Ok((meta, blobs))
}

/// Drops copies, oldest checkpoint first, until `frames` hold at most `cap` bytes, and whole
/// checkpoints past `MAX_CHECKPOINTS`. Returns the frames as bytes in order. A dropped copy
/// keeps its path and hash, so a later diff still says the file changed.
fn trimmed(
    f: &mut std::fs::File,
    mut frames: Vec<Frame>,
    newest: (Meta, Vec<u8>),
    cap: u64,
) -> io::Result<Vec<u8>> {
    let excess = (frames.len() + 1).saturating_sub(MAX_CHECKPOINTS);
    frames.drain(..excess);
    let mut total = held(&newest.0) + frames.iter().map(|x| held(&x.meta)).sum::<u64>();
    let mut stripped = vec![false; frames.len()];
    for (i, fr) in frames.iter().enumerate() {
        if total <= cap {
            break;
        }
        total -= held(&fr.meta);
        stripped[i] = true;
    }
    let mut encoded = Vec::with_capacity(frames.len() + 1);
    for (fr, strip) in frames.iter().zip(&stripped) {
        if *strip {
            let mut m = fr.meta.clone();
            m.files.iter_mut().for_each(|x| x.copy = None);
            encoded.push(encode(&m, &[])?);
        } else {
            let (m, blobs) = repack(f, fr)?;
            encoded.push(encode(&m, &blobs)?);
        }
    }
    let fitted = if total > cap { bare(newest.0) } else { newest };
    encoded.push(encode(&fitted.0, &fitted.1)?);
    // The reader bounds the headers of the whole log, so the oldest checkpoints go whole until
    // what is kept can be read back. The newest always stays.
    let sizes: Vec<u64> = encoded.iter().map(|b| hdr_of(b)).collect();
    let drop = oldest_to_drop(&sizes);
    Ok(encoded.drain(drop..).flatten().collect())
}

/// `frame` without its copies: every path and hash stays.
fn bare(mut meta: Meta) -> (Meta, Vec<u8>) {
    meta.files.iter_mut().for_each(|x| x.copy = None);
    (meta, Vec::new())
}

/// The frame as bytes, without its copies when they pass `cap`.
fn fit(frame: (Meta, Vec<u8>), cap: u64) -> io::Result<Vec<u8>> {
    let (meta, blobs) = if held(&frame.0) > cap { bare(frame.0) } else { frame };
    encode(&meta, &blobs)
}

impl Index {
    /// Records `files` as a checkpoint at `now` and returns its id and whether it is new. The
    /// same state as the newest checkpoint returns its id; one that matches only an older
    /// checkpoint is recorded again, and the id then names the newest of them. Copies past
    /// `cap_bytes` drop oldest first.
    ///
    /// # Errors
    /// When the cache cannot be written: a newer build owns it, or the disk refuses.
    pub fn checkpoint_add(
        &self,
        snap: &Snapshot,
        head: &str,
        auto: bool,
        label: &str,
        cap_bytes: u64,
        now: i64,
    ) -> io::Result<(String, bool)> {
        self.writable()?;
        let cap_bytes = cap_bytes.min(MAX_BLOB);
        // The cache's writer lock: a rewrite replaces the file, so it must not run between
        // another writer's scan and append, in this process or another.
        let _lock = crate::store::WriteLock::take(self.dir())?;
        let (mut frames, valid) = match cache::open_read(self.dir(), FILE)? {
            Some(mut f) => {
                let found = scan(&mut f)?;
                if found.damaged {
                    // A rewrite would drop what this build cannot read. That is the user's
                    // call, not ours.
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the checkpoints file holds data this build cannot read; move or delete \
                         .git/gitterrace/checkpoints to start a new one",
                    ));
                }
                (Some((f, found.frames)), found.valid)
            }
            None => (None, 0),
        };
        let id = id_of(&snap.files, head);
        // Only the newest frame stands for "nothing changed since the last checkpoint". A state
        // that matches an older frame (A, then B, then A again) is a new checkpoint, so the
        // newest one is always the state the user last saved, whatever the cap dropped.
        if frames.as_ref().and_then(|(_, fr)| fr.last()).is_some_and(|x| x.meta.id == id) {
            return Ok((id, false));
        }
        let (meta, blobs) = build(snap, head, auto, label, now, cap_bytes)?;
        let Some((f, fr)) = frames.as_mut() else {
            // The first frame meets the same cap as every later one.
            let bytes = fit((meta, blobs), cap_bytes)?;
            cache::write_atomic(self.dir(), FILE, &bytes)?;
            return Ok((id, true));
        };
        let size = f.metadata()?.len();
        let total = held(&meta) + fr.iter().map(|x| held(&x.meta)).sum::<u64>();
        let bytes = encode(&meta, &blobs)?;
        let headers = hdr_of(&bytes) + fr.iter().map(|x| u64::from(x.hdr_len)).sum::<u64>();
        if valid == size
            && total <= cap_bytes
            && headers <= MAX_HEADERS
            && fr.len() < MAX_CHECKPOINTS
        {
            use std::io::Write;
            let mut out = cache::open_append(self.dir(), FILE)?;
            out.write_all(&bytes)?;
            out.sync_data()?;
        } else {
            let all = std::mem::take(fr);
            let bytes = trimmed(f, all, (meta, blobs), cap_bytes)?;
            cache::write_atomic(self.dir(), FILE, &bytes)?;
        }
        Ok((id, true))
    }

    /// The checkpoints, newest first. A missing or empty log lists nothing.
    ///
    /// # Errors
    /// When the file cannot be read.
    pub fn checkpoint_list(&self) -> io::Result<CheckpointList> {
        let Some(mut f) = cache::open_read(self.dir(), FILE)? else {
            return Ok(CheckpointList::default());
        };
        let frames = scan(&mut f)?.frames;
        let items: Vec<CheckpointInfo> = frames
            .iter()
            .rev()
            .map(|x| CheckpointInfo {
                id: x.meta.id.clone(),
                created: x.meta.created,
                head: x.meta.head.clone(),
                files: x.meta.files.len(),
                bytes: held(&x.meta),
                auto: x.meta.auto,
                label: x.meta.label.clone(),
                files_omitted: x.meta.omitted,
            })
            .collect();
        let bytes_used = items.iter().map(|i| i.bytes).sum();
        Ok(CheckpointList { items, bytes_used })
    }

    /// The files of checkpoint `id` with their copies, `None` for an unknown id. A copy that is
    /// gone or fails its checksum reads as no copy, and the caller then diffs against HEAD.
    ///
    /// # Errors
    /// When the file cannot be read.
    pub fn checkpoint_files(&self, id: &str) -> io::Result<Option<Vec<SnapFile>>> {
        Ok(self.checkpoint(id)?.map(|c| c.files))
    }

    /// Checkpoint `id`: the HEAD it was taken at and its files with their copies, `None` for an
    /// unknown id. When a state was taken twice the newest frame answers.
    ///
    /// # Errors
    /// When the file cannot be read.
    pub fn checkpoint(&self, id: &str) -> io::Result<Option<Checkpoint>> {
        let Some(mut f) = cache::open_read(self.dir(), FILE)? else {
            return Ok(None);
        };
        let frames = scan(&mut f)?.frames;
        let Some(fr) = frames.iter().rev().find(|x| x.meta.id == id) else {
            return Ok(None);
        };
        let mut left = MAX_RESTORE;
        let mut files = Vec::with_capacity(fr.meta.files.len());
        for m in &fr.meta.files {
            let text = match &m.copy {
                Some(c) if c.size <= left => {
                    let text = copy_of(&mut f, fr, c)?;
                    if text.is_some() {
                        left -= c.size;
                    }
                    text
                }
                _ => None,
            };
            files.push(SnapFile { path: m.path.clone(), hash: m.hash.clone(), text });
        }
        Ok(Some(Checkpoint { head: fr.meta.head.clone(), files_omitted: fr.meta.omitted, files }))
    }
}

/// One copy unpacked, `None` when it fails its checksum or does not unpack to its stated size.
fn copy_of(f: &mut std::fs::File, frame: &Frame, c: &Copy) -> io::Result<Option<Vec<u8>>> {
    let Some(z) = packed(f, frame, c)? else { return Ok(None) };
    let size = usize::try_from(c.size).map_err(io::Error::other)?;
    Ok(zstd::bulk::decompress(&z, size).ok())
}

#[cfg(test)]
mod tests;
