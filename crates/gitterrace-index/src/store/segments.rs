//! Builds, appends and merges the segment set, and the shallow and oid helpers around it.

use std::io;
use std::path::Path;

use crate::CommitRecord;
use crate::store::{WriteLock, check_writable, wipe};

/// A fresh build of the segment set: records arrive in canonical order,
/// the aggregates and postings fold as they come, `finish` writes one segment, its string
/// delta and the order file, then swaps them in with a new manifest. Before the manifest
/// rename none of it exists for readers, and a crash leaves strays the open sweep removes.
pub(crate) struct SegmentBuild {
    dir: std::path::PathBuf,
    generation: u64,
    strings: crate::seg::agg::Strings,
    labels: crate::seg::agg::Interner,
    builder: crate::feed::segments::SegmentBuilder,
    rows: crate::feed::RowWriter,
    oids: Vec<crate::seg::Oid>,
    shallow: u64,
}

impl SegmentBuild {
    /// Starts the build after `previous`, whose files it retires on finish. `shallow` is the
    /// clone's shallow boundary (`.git/shallow`), recorded so a deepened clone rebuilds. The
    /// spill's run file lives in the cache directory under a temp name, so a crash leaves only
    /// a stray.
    pub(crate) fn new(
        dir: &Path,
        previous: Option<&crate::seg::manifest::Manifest>,
        shallow: &[String],
    ) -> crate::seg::SegResult<Self> {
        let generation = previous.map_or(0, |m| m.generation) + 1;
        let spill = dir.join(format!("spill-{generation}.gts.tmp"));
        let builder = crate::feed::segments::SegmentBuilder::new(move || {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&spill)
        });
        let shallow = shallow_of(shallow)?;
        let rows = crate::feed::RowWriter::new(dir, generation)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            generation,
            strings: crate::seg::agg::Strings::default(),
            labels: crate::seg::agg::Interner::default(),
            builder,
            rows,
            oids: Vec::new(),
            shallow,
        })
    }

    /// Folds the next record. Records must come in canonical order: the rank is the position.
    pub(crate) fn push(&mut self, r: &CommitRecord) -> crate::seg::SegResult<()> {
        let oid = parse_oid(&r.id)?;
        let rank = u32::try_from(self.oids.len()).map_err(|_| too_many())?;
        fold(
            &mut self.strings,
            &mut self.labels,
            &mut self.builder,
            &mut self.rows,
            r,
            rank,
        )?;
        self.oids.push(oid);
        Ok(())
    }

    /// Writes the segment, its string delta and the order file, then the manifest naming
    /// them plus `extra` (plain files such as `lines.json`, already written). Files of
    /// `previous` that are not live again are retired. String deltas of a retired segment
    /// stay live only in a partial rebuild, never in a fresh one, which renumbers all.
    pub(crate) fn finish(
        self,
        previous: Option<crate::seg::manifest::Manifest>,
        extra: Vec<crate::seg::manifest::FileEntry>,
    ) -> crate::seg::SegResult<crate::seg::manifest::Manifest> {
        use crate::seg::format::{Kind, StringBase};
        use crate::seg::strings::{StringDelta, write_delta};
        use crate::seg::write::{ContainerWriter, write_order};

        let g = self.generation;
        let seg_name = format!("seg-{g}-0.gts");
        let mut w =
            ContainerWriter::new(&self.dir, &seg_name, Kind::Segment, StringBase::default())?;
        let mut rows = self.rows;
        rows.commits(&mut |sec, items, payload| w.block(sec, items, payload))?;
        let paths = self.strings.paths.list.len();
        self.builder.finish(paths, &mut |sec, items, payload| {
            w.block(sec, items, payload)
        })?;
        rows.finish(&mut |sec, items, payload| w.block(sec, items, payload))?;
        let seg = w.finish()?;
        drop(std::fs::remove_file(
            self.dir.join(format!("spill-{g}.gts.tmp")),
        ));
        let owned = |l: Vec<Box<str>>| l.into_iter().map(String::from).collect();
        let delta = StringDelta {
            paths: owned(self.strings.paths.list),
            identities: owned(self.strings.identities.list),
            labels: owned(self.labels.list),
        };
        let (strings, _) = write_delta(
            &self.dir,
            &format!("str-{g}-0.gts"),
            StringBase::default(),
            &delta,
        )?;
        let order = write_order(&self.dir, &format!("ord-{g}.gts"), &self.oids)?;
        let mut live = vec![seg, strings, order];
        live.extend(extra);
        let mut retired = previous
            .map(|m| {
                m.retired
                    .into_iter()
                    .chain(m.live.into_iter().map(|f| f.name))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        retired.retain(|n| !live.iter().any(|f| f.name == *n));
        let mut m = crate::seg::manifest::Manifest {
            generation: g,
            fingerprint: crate::seg::fingerprint(&self.oids),
            commits: u32::try_from(self.oids.len()).map_err(|_| too_many())?,
            shallow: self.shallow,
            live,
            retired,
        };
        m.commit(&self.dir)?;
        Ok(m)
    }
}

/// Segments in a set before [`merge_if_needed`] folds them into one, so open files stay bounded.
/// Not configurable: a storage layout bound on open files, not a user-visible limit.
pub(crate) const MAX_SEGMENTS: usize = 8;

/// Folds one record into a segment under construction: labels, aggregates and postings, then
/// its row (sections 34 to 36) with the same global ids. `Strings::ids` interns, so calling it
/// here before the builder does gives the ids the builder will see.
fn fold(
    strings: &mut crate::seg::agg::Strings,
    labels: &mut crate::seg::agg::Interner,
    builder: &mut crate::feed::segments::SegmentBuilder,
    rows: &mut crate::feed::RowWriter,
    r: &CommitRecord,
    rank: u32,
) -> crate::seg::SegResult<()> {
    let kind = r.kind.as_deref().map(|l| labels.id(l));
    let scope = r.scope.as_deref().map(|l| labels.id(l));
    let (who, paths) = strings.ids(r);
    let olds: Vec<Option<u32>> = r
        .files
        .iter()
        .map(|f| f.old.as_deref().and_then(|o| strings.paths.get(o)))
        .collect();
    builder.push(strings, r, rank)?;
    rows.push(r, who, kind, scope, &paths, &olds)
}

/// Appends the commits `records` (new since the last build, in canonical order among
/// themselves) as one more segment on top of `previous`, so a HEAD move costs only its new commits.
///
/// String ids continue the previous deltas (ids stay global until a merge), ordinals are local to
/// the new segment, and its ranks follow the previous order file. The new order file is the old
/// order then the new commits: a valid topological order, not the canonical one, which only a
/// merge restores. Previous segments and deltas stay live, the old order file is retired.
pub(crate) fn append_segment<'r>(
    dir: &Path,
    previous: crate::seg::manifest::Manifest,
    shallow: &[String],
    records: impl IntoIterator<Item = &'r CommitRecord>,
) -> crate::seg::SegResult<crate::seg::manifest::Manifest> {
    use crate::seg::format::Kind;
    use crate::seg::strings::{StringDelta, write_delta};
    use crate::seg::write::{ContainerWriter, write_order};

    let store = crate::seg::read::Store::open(dir)?
        .ok_or_else(|| crate::seg::SegError::unreadable("MANIFEST", 0, "append without a set"))?;
    let (mut strings, mut labels, base) = load_strings(&store)?;
    let mut oids = load_order(&store)?;
    let g = previous.generation + 1;
    let n = store.segments.len();
    let spill = dir.join(format!("spill-{g}.gts.tmp"));
    let spill_open = spill.clone();
    let mut builder = crate::feed::segments::SegmentBuilder::new(move || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&spill_open)
    });
    let (paths0, idents0, labels0) = (
        strings.paths.list.len(),
        strings.identities.list.len(),
        labels.list.len(),
    );
    builder.continue_lives(load_lives(&store, paths0)?);
    let mut rows = crate::feed::RowWriter::new(dir, g)?;
    for r in records {
        let rank = u32::try_from(oids.len()).map_err(|_| too_many())?;
        fold(&mut strings, &mut labels, &mut builder, &mut rows, r, rank)?;
        oids.push(parse_oid(&r.id)?);
    }
    let seg_name = format!("seg-{g}-{n}.gts");
    let mut w = ContainerWriter::new(dir, &seg_name, Kind::Segment, base)?;
    rows.commits(&mut |sec, items, payload| w.block(sec, items, payload))?;
    let paths = strings.paths.list.len();
    builder.finish(paths, &mut |sec, items, payload| {
        w.block(sec, items, payload)
    })?;
    rows.finish(&mut |sec, items, payload| w.block(sec, items, payload))?;
    let seg = w.finish()?;
    drop(std::fs::remove_file(&spill));
    let tail = |l: &[Box<str>], from: usize| l[from..].iter().map(|s| String::from(&**s)).collect();
    let delta = StringDelta {
        paths: tail(&strings.paths.list, paths0),
        identities: tail(&strings.identities.list, idents0),
        labels: tail(&labels.list, labels0),
    };
    let (str_entry, _) = write_delta(dir, &format!("str-{g}-{n}.gts"), base, &delta)?;
    let order = write_order(dir, &format!("ord-{g}.gts"), &oids)?;
    drop(store);
    // Live order: segments then their deltas in segment order, then the order file, then the
    // plain files: the reader checks string bases in this order.
    let kind = |k: Kind| k as u8;
    let mut segs: Vec<_> = previous
        .live
        .iter()
        .filter(|f| f.kind == kind(Kind::Segment))
        .cloned()
        .collect();
    let mut strs: Vec<_> = previous
        .live
        .iter()
        .filter(|f| f.kind == kind(Kind::Strings))
        .cloned()
        .collect();
    // Plain files, then the message column, whose files stay live: the column step after the
    // append adds the new ranks.
    let plain: Vec<_> = previous
        .live
        .iter()
        .filter(|f| f.kind == 0 || f.kind == kind(Kind::Messages))
        .cloned()
        .collect();
    let mut retired = previous.retired;
    retired.extend(
        previous
            .live
            .iter()
            .filter(|f| f.kind == kind(Kind::Order))
            .map(|f| f.name.clone()),
    );
    segs.push(seg);
    strs.push(str_entry);
    let mut live = segs;
    live.extend(strs);
    live.push(order);
    live.extend(plain);
    let mut m = crate::seg::manifest::Manifest {
        generation: g,
        fingerprint: crate::seg::fingerprint(&oids),
        commits: u32::try_from(oids.len()).map_err(|_| too_many())?,
        shallow: shallow_of(shallow)?,
        live,
        retired,
    };
    m.commit(dir)?;
    Ok(m)
}

/// Folds the set into one segment once it holds more than [`MAX_SEGMENTS`], renumbered into
/// the canonical order `canonical` (every reachable commit id, as
/// `gitterrace_git::canonical_order` gives them). `record` fetches one commit's record by id.
///
/// The merge is a fresh build over the records in canonical order, so ordinals, ranks and
/// string ids come out exactly as a one-step build numbers them, byte for byte. Rebuilding from
/// records rather than renumbering the tables is required: the tables leave out the rows of
/// merge commits, yet those rows take part in first-use string numbering.
///
/// Crash safe: the new files are invisible until the manifest rename, the old set is retired
/// only after it. Serving is never blocked: readers take no lock and keep their open files
/// (on Windows a busy retired file is retried at the next sweep). Takes the writer lock, so
/// run it on a background thread. Returns whether it merged.
pub(crate) fn merge_if_needed(
    dir: &Path,
    canonical: &[String],
    shallow: &[String],
    extra: Vec<crate::seg::manifest::FileEntry>,
    record: &mut dyn FnMut(&str) -> io::Result<CommitRecord>,
) -> io::Result<bool> {
    let _lock = WriteLock::take(dir)?;
    check_writable(dir)?;
    let Some(previous) = crate::seg::manifest::Manifest::load(dir).map_err(io::Error::other)?
    else {
        return Ok(false);
    };
    let segments = previous
        .live
        .iter()
        .filter(|f| f.kind == crate::seg::format::Kind::Segment as u8)
        .count();
    if segments <= MAX_SEGMENTS {
        return Ok(false);
    }
    // The column is rewritten in the new order from itself: no git read, and byte for byte what
    // a fresh build writes. A partial column is left behind, and the next update rebuilds it.
    let mut extra = extra;
    let store = crate::seg::read::Store::open(dir)
        .map_err(crate::seg_io)?
        .ok_or_else(|| io::Error::other("merge without a set"))?;
    let name = format!("msg-{}-0.gts", previous.generation + 1);
    if let Some(col) =
        crate::messages::remap(dir, &name, store, canonical).map_err(crate::seg_io)?
    {
        extra.push(col);
    }
    let mut b = SegmentBuild::new(dir, Some(&previous), shallow).map_err(io::Error::other)?;
    for id in canonical {
        b.push(&record(id)?).map_err(io::Error::other)?;
    }
    b.finish(Some(previous), extra).map_err(io::Error::other)?;
    Ok(true)
}

/// Every live string delta read whole into interners, for an append that continues the ids.
/// Returns the paths and identities, the labels, and the cumulative base for the next delta.
fn load_strings(
    store: &crate::seg::read::Store,
) -> crate::seg::SegResult<(
    crate::seg::agg::Strings,
    crate::seg::agg::Interner,
    crate::seg::format::StringBase,
)> {
    use crate::seg::format::section::{IDENTITIES, LABELS, PATHS};

    let mut strings = crate::seg::agg::Strings::default();
    let mut labels = crate::seg::agg::Interner::default();
    let mut buf = Vec::new();
    for d in &store.strings {
        for (sec, into) in [
            (PATHS, &mut strings.paths),
            (IDENTITIES, &mut strings.identities),
            (LABELS, &mut labels),
        ] {
            for e in d.blocks(sec) {
                let block = d.read_block(e, &mut buf)?;
                let mut c = crate::seg::bounds::Cursor::new(block, d.name(), e.offset);
                for _ in 0..e.items {
                    let n = usize::from(c.u16()?);
                    let s =
                        std::str::from_utf8(c.bytes(n)?).map_err(|_| c.err("string not utf-8"))?;
                    into.id(s);
                }
            }
        }
    }
    let count = |l: usize| u32::try_from(l).map_err(|_| too_many());
    let base = crate::seg::format::StringBase {
        paths: count(strings.paths.list.len())?,
        identities: count(strings.identities.list.len())?,
        labels: count(labels.list.len())?,
    };
    Ok((strings, labels, base))
}

/// The life state in the newest segment's section 7, written against `paths` path ids. An
/// append continues it, so no life written before is renumbered. Read whole: a write.
fn load_lives(
    store: &crate::seg::read::Store,
    paths: usize,
) -> crate::seg::SegResult<crate::seg::canon::Lives> {
    let Some(seg) = store.segments.last() else {
        return Ok(crate::seg::canon::Lives::default());
    };
    let mut blocks = Vec::new();
    let mut buf = Vec::new();
    for e in seg.blocks(crate::seg::format::section::CANON) {
        blocks.push(seg.read_block(e, &mut buf)?.to_vec());
    }
    let refs: Vec<&[u8]> = blocks.iter().map(Vec::as_slice).collect();
    crate::seg::canon::Lives::decode(&refs, paths, seg.name())
}

/// The order file's oids, in order.
pub(crate) fn load_order(
    store: &crate::seg::read::Store,
) -> crate::seg::SegResult<Vec<crate::seg::Oid>> {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    if let Some(o) = &store.order {
        for e in o.blocks(crate::seg::format::section::ORDER) {
            let block = o.read_block(e, &mut buf)?;
            let mut c = crate::seg::bounds::Cursor::new(block, o.name(), e.offset);
            for _ in 0..e.items {
                out.push(c.array::<20>()?);
            }
        }
    }
    Ok(out)
}

fn shallow_of(ids: &[String]) -> crate::seg::SegResult<u64> {
    let oids = ids
        .iter()
        .map(|id| parse_oid(id))
        .collect::<crate::seg::SegResult<Vec<_>>>()?;
    Ok(crate::seg::manifest::shallow_fingerprint(&oids))
}

/// Whether the set was built against another shallow boundary than `shallow`: then every
/// commit is pending, since the next update rebuilds the set whole.
pub(crate) fn shallow_stale(dir: &Path, shallow: &[String]) -> bool {
    match crate::seg::manifest::Manifest::load(dir) {
        Ok(Some(m)) => shallow_of(shallow).map_or(true, |s| s != m.shallow),
        _ => false,
    }
}

/// Wipes the cache when the clone's shallow boundary is not the one its segments were built
/// against: deepening or grafting changes parents, so topo ranks and the order are stale.
/// Treated exactly as an unreadable cache, never partly served. Call before an update, under no
/// lock. A newer build's cache is left alone. Returns whether it wiped.
pub(crate) fn check_shallow(dir: &Path, shallow: &[String]) -> io::Result<bool> {
    let stale = |dir: &Path| shallow_stale(dir, shallow);
    if !stale(dir) {
        return Ok(false);
    }
    let _lock = WriteLock::take(dir)?;
    check_writable(dir)?;
    if stale(dir) {
        wipe(dir)?;
        return Ok(true);
    }
    Ok(false)
}

/// The oid of a hex commit id, `None` when it is not one.
pub(crate) fn oid_of(id: &str) -> Option<crate::seg::Oid> {
    parse_oid(id).ok()
}

fn parse_oid(id: &str) -> crate::seg::SegResult<crate::seg::Oid> {
    let bad = || crate::seg::SegError::unreadable("commit id", 0, "not a 40-digit hex oid");
    let b = id.as_bytes();
    if b.len() != 40 {
        return Err(bad());
    }
    let mut out = [0u8; 20];
    for (i, pair) in b.as_chunks::<2>().0.iter().enumerate() {
        let s = std::str::from_utf8(pair).map_err(|_| bad())?;
        out[i] = u8::from_str_radix(s, 16).map_err(|_| bad())?;
    }
    Ok(out)
}

const fn too_many() -> crate::seg::SegError {
    crate::seg::SegError::TooLong {
        kind: "commit count",
        len: usize::MAX,
        max: u32::MAX as usize,
    }
}
