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

use gitterrace_git::{SnapFile, content_id};
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
/// Largest copy read back, so a planted size cannot ask for gigabytes.
const MAX_COPY: u64 = 64 << 20;
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
}

/// The checkpoints, newest first, and what they hold in all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckpointList {
    pub items: Vec<CheckpointInfo>,
    pub bytes_used: u64,
}

/// A frame found in the log: where its blobs start and how long they are.
struct Frame {
    meta: Meta,
    blob_at: u64,
    blob_len: u64,
}

/// The id of a state: `cp-` and 12 hex of the git blob id of its manifest (paths and content
/// hashes), so an identical state is the same checkpoint.
fn id_of(files: &[SnapFile]) -> String {
    let mut rows: Vec<String> = files.iter().map(|f| format!("{}\0{}\n", f.path, f.hash)).collect();
    rows.sort();
    let id = content_id(rows.concat().as_bytes());
    format!("cp-{}", &id[..12])
}

const fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

const fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7]])
}

/// The whole frames of the log and where the last one ends. Anything after that is a torn tail.
fn scan(f: &mut std::fs::File) -> io::Result<(Vec<Frame>, u64)> {
    let len = f.metadata()?.len();
    let mut out = Vec::new();
    let mut at = 0;
    while at + FIXED <= len {
        f.seek(SeekFrom::Start(at))?;
        let mut fixed = [0u8; FIXED as usize];
        f.read_exact(&mut fixed)?;
        let (hdr_len, hdr_crc, blob_len) =
            (u32_at(&fixed, 4), u32_at(&fixed, 8), u64_at(&fixed, 12));
        if &fixed[..4] != MAGIC || hdr_len > MAX_HEADER {
            break;
        }
        let end = at + FIXED + u64::from(hdr_len) + blob_len;
        if end > len {
            break;
        }
        let mut hdr = vec![0; hdr_len as usize];
        f.read_exact(&mut hdr)?;
        if crc32fast::hash(&hdr) != hdr_crc {
            break;
        }
        let Ok(meta) = serde_json::from_slice::<Meta>(&hdr) else {
            break;
        };
        out.push(Frame { meta, blob_at: at + FIXED + u64::from(hdr_len), blob_len });
        at = end;
    }
    Ok((out, at))
}

fn blob_of(f: &mut std::fs::File, frame: &Frame) -> io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(frame.blob_at))?;
    let mut b = vec![0; usize::try_from(frame.blob_len).map_err(io::Error::other)?];
    f.read_exact(&mut b)?;
    Ok(b)
}

fn encode(meta: &Meta, blobs: &[u8]) -> io::Result<Vec<u8>> {
    let hdr = serde_json::to_vec(meta).map_err(io::Error::other)?;
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
    meta.files.iter().filter_map(|f| f.copy.as_ref()).map(|c| c.len).sum()
}

/// A checkpoint of `files` as one frame.
fn build(
    files: &[SnapFile],
    head: &str,
    auto: bool,
    label: &str,
    created: i64,
) -> io::Result<(Meta, Vec<u8>)> {
    let mut blobs = Vec::new();
    let mut metas = Vec::with_capacity(files.len());
    for f in files {
        let copy = match &f.text {
            Some(t) => {
                let z = zstd::bulk::compress(t, ZSTD_LEVEL)?;
                let c = Copy {
                    off: blobs.len() as u64,
                    len: z.len() as u64,
                    size: t.len() as u64,
                    crc: crc32fast::hash(&z),
                };
                blobs.extend_from_slice(&z);
                Some(c)
            }
            None => None,
        };
        metas.push(FileMeta { path: f.path.clone(), hash: f.hash.clone(), copy });
    }
    let meta = Meta {
        id: id_of(files),
        created,
        head: head.to_owned(),
        auto,
        label: label.chars().take(200).collect(),
        files: metas,
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
    let mut out = Vec::new();
    for (fr, strip) in frames.iter().zip(&stripped) {
        if *strip {
            let mut m = fr.meta.clone();
            m.files.iter_mut().for_each(|x| x.copy = None);
            out.extend(encode(&m, &[])?);
        } else {
            let blobs = blob_of(f, fr)?;
            out.extend(encode(&fr.meta, &blobs)?);
        }
    }
    let (mut meta, mut blobs) = newest;
    if total > cap {
        meta.files.iter_mut().for_each(|x| x.copy = None);
        blobs.clear();
    }
    out.extend(encode(&meta, &blobs)?);
    Ok(out)
}

impl Index {
    /// Records `files` as a checkpoint at `now` and returns its id and whether it is new. The
    /// same state again returns the existing id. Copies past `cap_bytes` drop oldest first.
    ///
    /// # Errors
    /// When the cache cannot be written: a newer build owns it, or the disk refuses.
    pub fn checkpoint_add(
        &self,
        files: &[SnapFile],
        head: &str,
        auto: bool,
        label: &str,
        cap_bytes: u64,
        now: i64,
    ) -> io::Result<(String, bool)> {
        self.writable()?;
        // The cache's writer lock: a rewrite replaces the file, so it must not run between
        // another writer's scan and append, in this process or another.
        let _lock = crate::store::WriteLock::take(self.dir())?;
        let (mut frames, valid) = match cache::open_read(self.dir(), FILE)? {
            Some(mut f) => {
                let (frames, valid) = scan(&mut f)?;
                (Some((f, frames)), valid)
            }
            None => (None, 0),
        };
        let id = id_of(files);
        if frames.as_ref().is_some_and(|(_, fr)| fr.iter().any(|x| x.meta.id == id)) {
            return Ok((id, false));
        }
        let (meta, blobs) = build(files, head, auto, label, now)?;
        let Some((f, fr)) = frames.as_mut() else {
            cache::write_atomic(self.dir(), FILE, &encode(&meta, &blobs)?)?;
            return Ok((id, true));
        };
        let size = f.metadata()?.len();
        let total = held(&meta) + fr.iter().map(|x| held(&x.meta)).sum::<u64>();
        if valid == size && total <= cap_bytes && fr.len() < MAX_CHECKPOINTS {
            use std::io::Write;
            let mut out = cache::open_append(self.dir(), FILE)?;
            out.write_all(&encode(&meta, &blobs)?)?;
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
        let (frames, _) = scan(&mut f)?;
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
        let Some(mut f) = cache::open_read(self.dir(), FILE)? else {
            return Ok(None);
        };
        let (frames, _) = scan(&mut f)?;
        let Some(fr) = frames.iter().find(|x| x.meta.id == id) else {
            return Ok(None);
        };
        let blobs = blob_of(&mut f, fr)?;
        let files = fr
            .meta
            .files
            .iter()
            .map(|m| SnapFile {
                path: m.path.clone(),
                hash: m.hash.clone(),
                text: m.copy.as_ref().and_then(|c| copy_of(&blobs, c)),
            })
            .collect();
        Ok(Some(files))
    }
}

fn copy_of(blobs: &[u8], c: &Copy) -> Option<Vec<u8>> {
    let end = c.off.checked_add(c.len)?;
    let z = blobs.get(usize::try_from(c.off).ok()?..usize::try_from(end).ok()?)?;
    if c.size > MAX_COPY || crc32fast::hash(z) != c.crc {
        return None;
    }
    zstd::bulk::decompress(z, usize::try_from(c.size).ok()?).ok()
}

#[cfg(test)]
mod tests;
