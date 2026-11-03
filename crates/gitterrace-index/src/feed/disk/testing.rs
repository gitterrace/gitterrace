//! Writes a segment cache from records the way the build does, for guards and benches.

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
        rows.commits(&mut |sec, items, p| cw.block(sec, items, p))?;
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
