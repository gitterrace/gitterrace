//! The file rows of every commit keyed by the file's name at the indexed tip, written at index
//! time so the per-file pages (hotspots, ownership, coupling, code age) read one small item per
//! commit instead of every file row of history with renames followed.
//!
//! One file `tip-<gen>.gts` (container kind 6), named in the manifest. Section 50 holds one item
//! per rank, ascending: the row count (0 for a merge, whose rows the pages never count), then
//! per row in row order the tip name's path id, the raw churn (added plus removed) and the
//! whitespace-blind churn as a zigzag difference from the raw one. The tip name of a row is
//! what `insights::files::Follow` gives it: the rename aliases replayed newest first, so a name
//! that was deleted and reused keeps both files' rows, as the row read does. Identities, times,
//! bots, mailmap, periods and selections stay with the commit columns and apply on read.
//!
//! A tip file is read only for the set it was written for: its meta (section 51) holds the
//! manifest's order fingerprint and commit count. Every build, append and merge writes it
//! again before its manifest goes live. An append that renames nothing old copies the old
//! items and adds the new commits'. A rename in the new commits moves the old rows of the name
//! it renames: their tip id goes through the new aliases, which is exact because every new edge
//! is newer than every old row. A merge renumbers ranks and ids, so it rebuilds from the rows.

use std::io;
use std::path::Path;

use crate::feed::{InsightsSource, PathId, Rank, RenameEdge};
use crate::insights::files::Follow;
use crate::seg::bounds::{Cursor, MAX_BLOCK, put_varint, unzigzag, zigzag};
use crate::seg::format::{Kind, StringBase, section};
use crate::seg::manifest::{FileEntry, Manifest};
use crate::seg::read::{Container, Store};
use crate::seg::write::ContainerWriter;
use crate::seg::{SegError, SegResult};

/// Meta payload: fingerprint u64, commit count u32.
const META_LEN: usize = 12;

/// One row of a commit under its tip name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TipTouch {
    pub path: PathId,
    /// Added plus removed lines.
    pub raw: u32,
    /// The same with whitespace-only changes ignored.
    pub code: u32,
}

/// Writes the tip file of the set in `dir` when the live one is missing or stale. The caller
/// holds the writer lock.
pub(crate) fn sync(dir: &Path) -> io::Result<()> {
    let seg = crate::seg_io;
    let Some(store) = Store::open(dir).map_err(seg)? else {
        return Ok(());
    };
    if let Some(c) = &store.tip
        && current(c, &store.manifest).map_err(seg)?.is_some()
    {
        return Ok(());
    }
    let mut m = store.manifest.clone();
    drop(store);
    stage(dir, &mut m).map_err(seg)?;
    m.commit(dir).map_err(seg)
}

/// Writes the tip file of the set `m` names into `m` without publishing it: a live tip file
/// moves to `retired` and the new one is named. When a retired tip file of `m` covers a prefix
/// of the new order, its items are reused (see the module docs), else every row is read.
pub(crate) fn stage(dir: &Path, m: &mut Manifest) -> SegResult<()> {
    let kind = Kind::Tip as u8;
    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut m.live)
        .into_iter()
        .partition(|f| f.kind == kind);
    m.live = kept;
    m.retired.extend(gone.into_iter().map(|f| f.name));
    if m.commits == 0 {
        return Ok(());
    }
    let oids = {
        let store = Store::of(dir, m.clone())?;
        crate::store::load_order(&store)?
    };
    let old = previous(dir, m, &oids);
    let disk = crate::feed::DiskSource::open(Store::of(dir, m.clone())?, &mut |p| Ok(p.to_vec()))?;
    let tables = Store::of(dir, m.clone())?;
    let src = crate::feed::SegmentSource::new(disk, tables.segments);
    let name = format!("tip-{}.gts", m.generation);
    let entry = write(dir, &name, &src, old.as_ref(), m)?;
    if let Some(e) = src.failure() {
        return Err(SegError::unreadable(&name, 0, leak(e.0)));
    }
    let msg = Kind::Messages as u8;
    let at = m
        .live
        .iter()
        .position(|f| f.kind == msg)
        .unwrap_or(m.live.len());
    m.live.insert(at, entry);
    Ok(())
}

/// A source error's text as the static reason [`SegError::unreadable`] takes. Only reached when
/// a build met a broken cache, which then is deleted and rebuilt, so the leak is once per wipe.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// The newest retired tip file of `m` that covers a prefix of `oids`, with its commit count.
fn previous(dir: &Path, m: &Manifest, oids: &[crate::seg::Oid]) -> Option<(Container, u32)> {
    // Names are the writer's own `tip-<gen>.gts`, lower case (`check_name`).
    let generation = |n: &str| {
        n.strip_prefix("tip-")
            .and_then(|r| r.strip_suffix(".gts"))
            .and_then(|g| g.parse::<u64>().ok())
    };
    let mut names: Vec<&String> = m
        .retired
        .iter()
        .filter(|n| generation(n).is_some())
        .collect();
    names.sort_by_key(|n| generation(n));
    let name = names.pop()?;
    let c = Container::open(dir, name, Kind::Tip).ok()?;
    let (fp, commits) = meta(&c).ok()?;
    let n = commits as usize;
    (n > 0 && n <= oids.len() && crate::seg::fingerprint(&oids[..n]) == fp).then_some((c, commits))
}

/// Writes every item: the old file's items through the new aliases, then the new commits'
/// from their rows, or all from the rows when there is no old file.
fn write(
    dir: &Path,
    name: &str,
    src: &dyn InsightsSource,
    old: Option<&(Container, u32)>,
    m: &Manifest,
) -> SegResult<FileEntry> {
    let mut w = ContainerWriter::new(dir, name, Kind::Tip, StringBase::default())?;
    let edges: Vec<RenameEdge> = src.rename_edges().collect();
    let merge_at = |r: Rank| src.commit(r).is_some_and(|c| c.merge);
    let follow = Follow::of_edges(edges.clone(), merge_at);
    let mut out = Items::default();
    let from = match old {
        Some((c, n)) => {
            let fresh: Vec<RenameEdge> = edges.into_iter().filter(|e| e.rank.0 >= *n).collect();
            let moved = Follow::of_edges(fresh, merge_at);
            let paths = src.paths();
            let mut row = Vec::new();
            let mut each = |_: Rank, rows: &[TipTouch]| -> SegResult<()> {
                row.clear();
                // Every new edge is newer than this row, so it sees them all.
                row.extend(rows.iter().map(|t| TipTouch {
                    path: moved.canonical(t.path, Rank(0), 0),
                    ..*t
                }));
                out.push(&row, &mut w)
            };
            read(c, *n, paths, &mut each)?;
            *n
        }
        None => 0,
    };
    let ranks: Vec<Rank> = (from..m.commits).map(Rank).collect();
    let merges: Vec<bool> = src
        .commits_in(crate::feed::CommitRange::Ranks(from..m.commits))
        .map(|c| c.merge)
        .collect();
    let mut row = Vec::new();
    for (rank, files) in src.rows_for(ranks) {
        row.clear();
        if !merges
            .get((rank.0 - from) as usize)
            .copied()
            .unwrap_or(true)
        {
            for (i, f) in files.iter().enumerate() {
                let (ca, cd, _) = f.code();
                row.push(TipTouch {
                    path: follow.canonical(f.path, rank, u32::try_from(i).unwrap_or(u32::MAX)),
                    raw: f.add.saturating_add(f.del),
                    code: ca.saturating_add(cd),
                });
            }
        }
        out.push(&row, &mut w)?;
    }
    out.flush(&mut w)?;
    if out.total != m.commits {
        return Err(SegError::unreadable(
            name,
            0,
            "tip items do not cover the commits",
        ));
    }
    let mut meta = Vec::with_capacity(META_LEN);
    meta.extend_from_slice(&m.fingerprint.to_le_bytes());
    meta.extend_from_slice(&m.commits.to_le_bytes());
    w.block(section::TIP_META, 1, &meta)?;
    w.finish()
}

/// Items packed into blocks of at most [`MAX_BLOCK`].
#[derive(Default)]
struct Items {
    buf: Vec<u8>,
    item: Vec<u8>,
    in_block: u32,
    total: u32,
}

impl Items {
    fn push(&mut self, rows: &[TipTouch], w: &mut ContainerWriter) -> SegResult<()> {
        self.item.clear();
        encode(&mut self.item, rows);
        if self.item.len() > MAX_BLOCK {
            return Err(SegError::TooLong {
                kind: "tip item",
                len: self.item.len(),
                max: MAX_BLOCK,
            });
        }
        if self.buf.len() + self.item.len() > MAX_BLOCK {
            self.flush(w)?;
        }
        self.buf.extend_from_slice(&self.item);
        self.in_block += 1;
        self.total += 1;
        Ok(())
    }

    fn flush(&mut self, w: &mut ContainerWriter) -> SegResult<()> {
        if self.in_block > 0 {
            w.block(section::TIP_TOUCHES, self.in_block, &self.buf)?;
            self.buf.clear();
            self.in_block = 0;
        }
        Ok(())
    }
}

fn encode(out: &mut Vec<u8>, rows: &[TipTouch]) {
    put_varint(out, rows.len() as u64);
    for t in rows {
        put_varint(out, u64::from(t.path.0));
        put_varint(out, u64::from(t.raw));
        put_varint(out, zigzag(i64::from(t.raw) - i64::from(t.code)));
    }
}

/// Fingerprint and commit count from the meta. A missing or malformed meta, or one whose count
/// disagrees with the items, is `Unreadable`.
fn meta(c: &Container) -> SegResult<(u64, u32)> {
    let bad = |what| SegError::unreadable(c.name(), 0, what);
    let [e] = c.blocks(section::TIP_META) else {
        return Err(bad("tip meta missing or repeated"));
    };
    let mut buf = Vec::new();
    let b = c.read_block(e, &mut buf)?;
    if e.items != 1 || b.len() != META_LEN {
        return Err(bad("tip meta length"));
    }
    let mut cur = Cursor::new(b, c.name(), e.offset);
    let (fp, commits) = (cur.u64()?, cur.u32()?);
    let items: u64 = c
        .blocks(section::TIP_TOUCHES)
        .iter()
        .map(|e| u64::from(e.items))
        .sum();
    if items != u64::from(commits) {
        return Err(bad("tip item count"));
    }
    Ok((fp, commits))
}

/// The commit count when `c` was written for the set `m`, `None` when it is stale.
pub(crate) fn current(c: &Container, m: &Manifest) -> SegResult<Option<u32>> {
    let (fp, commits) = meta(c)?;
    Ok((fp == m.fingerprint && commits == m.commits).then_some(commits))
}

/// Hands each rank's rows to `each`, ascending from 0. Path ids must be below `paths` and
/// counts fit u32, else `Unreadable`.
pub(crate) fn read(
    c: &Container,
    commits: u32,
    paths: usize,
    each: &mut dyn FnMut(Rank, &[TipTouch]) -> SegResult<()>,
) -> SegResult<()> {
    let mut buf = Vec::new();
    let mut rows = Vec::new();
    let mut rank = 0u32;
    for e in c.blocks(section::TIP_TOUCHES) {
        let b = c.read_block(e, &mut buf)?;
        let mut cur = Cursor::new(b, c.name(), e.offset);
        for _ in 0..e.items {
            if rank >= commits {
                return Err(cur.err("tip item past the commits"));
            }
            rows.clear();
            item(&mut cur, paths, &mut rows)?;
            each(Rank(rank), &rows)?;
            rank += 1;
        }
        if !cur.is_empty() {
            return Err(cur.err("tip block past its items"));
        }
    }
    if rank != commits {
        return Err(SegError::unreadable(
            c.name(),
            0,
            "tip items short of the commits",
        ));
    }
    Ok(())
}

/// Decodes one item. Pushed one row at a time: a hostile count runs out of bytes, it never
/// sizes a buffer.
pub(crate) fn item(cur: &mut Cursor<'_>, paths: usize, rows: &mut Vec<TipTouch>) -> SegResult<()> {
    let n = cur.varint()?;
    for _ in 0..n {
        let path = cur.varint()?;
        if path >= paths as u64 {
            return Err(cur.err("tip path id past the strings"));
        }
        let raw = u32::try_from(cur.varint()?).map_err(|_| cur.err("tip churn"))?;
        let code = i64::from(raw) - unzigzag(cur.varint()?);
        let code = u32::try_from(code).map_err(|_| cur.err("tip code churn"))?;
        rows.push(TipTouch {
            path: PathId(path as u32),
            raw,
            code,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
