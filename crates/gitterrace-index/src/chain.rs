//! The first-parent chain of the indexed tip with each step's line counts, written at index
//! time so the languages page reads it in one sequential pass instead of walking the history
//! one commit record at a time.
//!
//! One file `chain-<gen>.gts` (container kind 5), named in the manifest. It holds the chain
//! from the newest rank (the commit the set was built for) along first parents: per step the
//! rank, oid, raw time and offset, and each file row's path id, added and removed lines, the
//! numbers the languages tally reads. Paths stay ids, so the language and the path filter
//! are decided at read time and an exclude, generated or ignore change needs no rebuild.
//!
//! A chain is read only for the set it was written for: its meta holds the manifest's order
//! fingerprint and commit count. An append, a merge or a rebuild changes the fingerprint, so
//! a chain that was not rewritten after one is stale and the page walks instead. A deepened
//! shallow clone wipes the whole cache before any of that.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::feed::{ChainStep, DiskSource, FileRow, PathId, Rank};
use crate::seg::bounds::{Cursor, MAX_BLOCK, put_varint};
use crate::seg::format::{Kind, StringBase, section};
use crate::seg::manifest::{FileEntry, Manifest};
use crate::seg::read::{Container, Store};
use crate::seg::write::ContainerWriter;
use crate::seg::{Oid, SegError, SegResult};

/// Meta payload: fingerprint u64, commit count u32, chain length u32.
const META_LEN: usize = 16;

/// Writes the chain of the set in `dir` when the live one is missing or stale, and retires
/// a stale one. No git read: steps and parents come from the stored rows. The caller holds
/// the writer lock.
pub(crate) fn sync(dir: &Path) -> io::Result<()> {
    let seg = crate::seg_io;
    let Some(store) = Store::open(dir).map_err(seg)? else {
        return Ok(());
    };
    if let Some(c) = &store.chain
        && current(c, &store.manifest).map_err(seg)?.is_some()
    {
        return Ok(());
    }
    let mut m = store.manifest.clone();
    drop(store);
    stage(dir, &mut m).map_err(seg)?;
    m.commit(dir).map_err(seg)
}

/// Writes the chain of the set `m` names into `m` without publishing it: a live chain moves
/// to `retired` and the new one is named. A merge calls this before its manifest goes live,
/// so no published set ever lacks the chain it is built with. On error `m` may have changed
/// but nothing is published, and the written file is a stray the sweep removes.
pub(crate) fn stage(dir: &Path, m: &mut Manifest) -> SegResult<()> {
    let kind = Kind::Chain as u8;
    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut m.live)
        .into_iter()
        .partition(|f| f.kind == kind);
    m.live = kept;
    m.retired.extend(gone.into_iter().map(|f| f.name));
    if m.commits == 0 {
        return Ok(());
    }
    let store = Store::of(dir, m.clone())?;
    let rank: HashMap<Oid, u32> = crate::store::load_order(&store)?
        .into_iter()
        .enumerate()
        .map(|(i, o)| (o, u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    let disk = DiskSource::open(store, &mut |p| Ok(p.to_vec()))?;
    let name = format!("chain-{}.gts", m.generation);
    let entry = write(dir, &name, &disk, &rank, m)?;
    // Before the message column, where a build puts it (the column is added after), so a merge
    // that carries the column names the files in the order a one-step build does.
    let msg = Kind::Messages as u8;
    let at = m
        .live
        .iter()
        .position(|f| f.kind == msg)
        .unwrap_or(m.live.len());
    m.live.insert(at, entry);
    Ok(())
}

/// Walks first parents from the newest rank, one stored record per step, and writes them.
fn write(
    dir: &Path,
    name: &str,
    disk: &DiskSource,
    rank: &HashMap<Oid, u32>,
    m: &Manifest,
) -> SegResult<FileEntry> {
    let mut w = ContainerWriter::new(dir, name, Kind::Chain, StringBase::default())?;
    let (mut buf, mut item, mut files) = (Vec::new(), Vec::new(), Vec::new());
    let (mut items, mut len) = (0u32, 0u32);
    let mut at = m.commits.checked_sub(1);
    while let Some(r) = at {
        files.clear();
        let (oid, time, offset, parent) = disk.chain_step(Rank(r), &mut files)?;
        item.clear();
        encode(&mut item, r, &oid, time, offset, &files);
        if item.len() > MAX_BLOCK {
            return Err(SegError::TooLong {
                kind: "chain step",
                len: item.len(),
                max: MAX_BLOCK,
            });
        }
        if buf.len() + item.len() > MAX_BLOCK {
            w.block(section::CHAIN, items, &buf)?;
            buf.clear();
            items = 0;
        }
        buf.extend_from_slice(&item);
        items += 1;
        len += 1;
        at = parent.and_then(|p| rank.get(&p).copied());
        // Canonical order puts every parent below its child, so the walk always ends.
        if at.is_some_and(|p| p >= r) {
            return Err(SegError::unreadable(
                name,
                0,
                "first parent not below its child",
            ));
        }
    }
    if items > 0 {
        w.block(section::CHAIN, items, &buf)?;
    }
    let mut meta = Vec::with_capacity(META_LEN);
    meta.extend_from_slice(&m.fingerprint.to_le_bytes());
    meta.extend_from_slice(&m.commits.to_le_bytes());
    meta.extend_from_slice(&len.to_le_bytes());
    w.block(section::CHAIN_META, 1, &meta)?;
    w.finish()
}

fn encode(out: &mut Vec<u8>, rank: u32, oid: &Oid, time: i64, offset: i32, files: &[FileRow]) {
    out.extend_from_slice(&rank.to_le_bytes());
    out.extend_from_slice(oid);
    out.extend_from_slice(&time.to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    put_varint(out, files.len() as u64);
    for f in files {
        put_varint(out, u64::from(f.path.0));
        put_varint(out, u64::from(f.add));
        put_varint(out, u64::from(f.del));
    }
}

/// The chain length when `c` was written for the set `m`, `None` when it is stale. A meta
/// block that is missing, malformed or disagrees with the step count is `Unreadable`.
pub(crate) fn current(c: &Container, m: &Manifest) -> SegResult<Option<u32>> {
    let bad = |what| SegError::unreadable(c.name(), 0, what);
    let [e] = c.blocks(section::CHAIN_META) else {
        return Err(bad("chain meta missing or repeated"));
    };
    let mut buf = Vec::new();
    let b = c.read_block(e, &mut buf)?;
    if e.items != 1 || b.len() != META_LEN {
        return Err(bad("chain meta length"));
    }
    let mut cur = Cursor::new(b, c.name(), e.offset);
    let (fp, commits, len) = (cur.u64()?, cur.u32()?, cur.u32()?);
    let steps: u64 = c
        .blocks(section::CHAIN)
        .iter()
        .map(|e| u64::from(e.items))
        .sum();
    if steps != u64::from(len) || len > commits || (len == 0) != (commits == 0) {
        return Err(bad("chain step count"));
    }
    Ok((fp == m.fingerprint && commits == m.commits).then_some(len))
}

/// Hands each step of `c` to `each`, newest first. See [`step`] for what is refused.
pub(crate) fn steps(
    c: &Container,
    commits: u32,
    paths: usize,
    latest: i64,
    each: &mut dyn FnMut(&ChainStep),
) -> SegResult<()> {
    let mut buf = Vec::new();
    let mut below = commits;
    for e in c.blocks(section::CHAIN) {
        let b = c.read_block(e, &mut buf)?;
        let mut cur = Cursor::new(b, c.name(), e.offset);
        for _ in 0..e.items {
            step(&mut cur, commits, &mut below, paths, latest, each)?;
        }
        if !cur.is_empty() {
            return Err(cur.err("chain block past its steps"));
        }
    }
    Ok(())
}

/// Decodes one step. `below` is the previous step's rank (`commits` before the first): the
/// first rank must be the newest and every next one lower. Path ids must be below `paths`
/// and counts fit u32, else `Unreadable`. Times are clamped to `latest` as commit rows are.
pub(crate) fn step(
    cur: &mut Cursor<'_>,
    commits: u32,
    below: &mut u32,
    paths: usize,
    latest: i64,
    each: &mut dyn FnMut(&ChainStep),
) -> SegResult<()> {
    let r = cur.u32()?;
    let first = *below == commits;
    if r >= *below || (first && r.checked_add(1) != Some(commits)) {
        return Err(cur.err("chain rank"));
    }
    *below = r;
    let oid = cur.array()?;
    let time = crate::plausible_time(i64::from_le_bytes(cur.array()?), latest);
    let offset = i32::from_le_bytes(cur.array()?);
    let n = cur.varint()?;
    let mut files = Vec::new();
    // Pushed one at a time: a hostile count runs out of bytes, it never sizes a buffer.
    for _ in 0..n {
        let path = cur.varint()?;
        if path >= paths as u64 {
            return Err(cur.err("chain path id past the strings"));
        }
        let add = u32::try_from(cur.varint()?).map_err(|_| cur.err("chain add"))?;
        let del = u32::try_from(cur.varint()?).map_err(|_| cur.err("chain del"))?;
        files.push(FileRow {
            path: PathId(path as u32),
            add,
            del,
            ..FileRow::default()
        });
    }
    each(&ChainStep {
        rank: Rank(r),
        oid,
        time,
        offset,
        files,
    });
    Ok(())
}

#[cfg(test)]
mod tests;
