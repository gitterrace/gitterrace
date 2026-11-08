//! Writes the message column: built or extended after an index update, rewritten by a merge.

use std::collections::HashMap;
use std::path::Path;

use crate::messages::{Item, MessageColumn, Writer, hex};
use crate::seg::manifest::FileEntry;
use crate::seg::read::Store;
use crate::seg::{Oid, SegError, SegResult};

/// The column rewritten in `canonical` order for a merge, `None` when `store` has no full
/// column. Parent ranks are renumbered through the oids, so the file is byte for byte the one
/// a fresh build writes.
pub(crate) fn remap(
    dir: &Path,
    name: &str,
    store: Store,
    canonical: &[String],
) -> SegResult<Option<FileEntry>> {
    let old_order = crate::store::load_order(&store)?;
    let Some(col) = MessageColumn::of(store)? else {
        return Ok(None);
    };
    let items = col.items()?;
    let old: HashMap<Oid, u32> = old_order
        .iter()
        .enumerate()
        .map(|(i, o)| (*o, u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    let new_oids: Vec<Oid> = canonical
        .iter()
        .map(|id| {
            crate::store::oid_of(id)
                .ok_or_else(|| SegError::unreadable("order", 0, "not a 40-digit hex oid"))
        })
        .collect::<SegResult<_>>()?;
    let new: HashMap<Oid, u32> = new_oids
        .iter()
        .enumerate()
        .map(|(i, o)| (*o, u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    let mut w = Writer::new(dir, name)?;
    for oid in &new_oids {
        let Some(&r) = old.get(oid) else {
            return Ok(None);
        };
        let mut it = items[r as usize].clone();
        it.parents = it
            .parents
            .iter()
            .filter_map(|p| new.get(&old_order[*p as usize]).copied())
            .collect();
        w.push(&it)?;
    }
    w.finish().map(Some)
}

/// Brings the column of the set in `dir` up to every rank: reads the texts of the ranks it
/// lacks from `source` and names one more file in the manifest. A source that cannot give
/// texts (a test double) leaves the column as it is. The caller holds the writer lock.
pub(crate) fn extend(dir: &Path, source: &dyn crate::CommitSource) -> std::io::Result<()> {
    let seg = crate::seg_io;
    let Some(store) = Store::open(dir).map_err(seg)? else {
        return Ok(());
    };
    let have: u64 = store.messages.iter().map(|f| u64::from(f.items())).sum();
    let commits = store.manifest.commits;
    if have >= u64::from(commits) {
        return Ok(());
    }
    let order = crate::store::load_order(&store).map_err(seg)?;
    let mut manifest = store.manifest.clone();
    let n = store.messages.len();
    drop(store);
    let rank: HashMap<Oid, u32> = order
        .iter()
        .enumerate()
        .map(|(i, o)| (*o, u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    let from = usize::try_from(have).unwrap_or(usize::MAX);
    let ids: Vec<String> = order
        .get(from..)
        .unwrap_or_default()
        .iter()
        .map(hex)
        .collect();
    let name = format!("msg-{}-{n}.gts", manifest.generation);
    let mut w = Writer::new(dir, &name).map_err(seg)?;
    let mut next = from;
    let mut failed = None;
    let supported = source.stream_texts(&ids, &mut |t| {
        // The texts come in the order asked, so each one is the next rank.
        if crate::store::oid_of(&t.id) != order.get(next).copied() {
            return Err(std::io::Error::other("commit text out of order"));
        }
        next += 1;
        if let Err(e) = w.push(&Item::of(t, &rank)) {
            failed = Some(e);
        }
        Ok(())
    })?;
    if let Some(e) = failed {
        return Err(seg(e));
    }
    if !supported || next != order.len() {
        return Ok(());
    }
    let entry = w.finish().map_err(seg)?;
    manifest.live.push(entry);
    manifest.commit(dir).map_err(seg)
}
