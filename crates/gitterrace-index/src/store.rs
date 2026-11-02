use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use crate::{CodeStat, CommitRecord, FileStat, cache};

/// Cache format; a different one on disk discards the records.
pub const FORMAT_VERSION: &str = "4";
pub(crate) const VERSION_FILE: &str = "VERSION";
pub(crate) const COMMITS_FILE: &str = "commits.jsonl";
pub(crate) const PATHS_FILE: &str = "paths.jsonl";
pub(crate) const LOCK_FILE: &str = "lock";

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if signature"
)]
const fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A record as stored: file paths are numbers into the paths file, so a path that thousands of
/// commits touch is written once. `id` stays the first field and `shallow` the second, see
/// [`stored_ids`]. Author and e-mail are as written in the commit: the `.mailmap` is applied
/// when records are read, so editing it never needs a rebuild.
#[derive(serde::Serialize, serde::Deserialize)]
struct Row {
    id: String,
    /// Diffed as a root because the clone was shallow there; rebuilt after an unshallow.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    shallow: bool,
    author: String,
    email: String,
    time: i64,
    offset: i32,
    merge: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parents: Vec<String>,
    kind: Option<String>,
    scope: Option<String>,
    files: Vec<FileRow>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FileRow {
    path: u32,
    additions: u32,
    deletions: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    lines: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    rewritten: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<CodeStat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    old: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    binary: bool,
}

/// The path table: ids are line numbers of the paths file.
type Paths = Vec<Arc<str>>;

/// Complete lines of the paths file, each a JSON string, and the byte length they span. A torn
/// last line is dropped.
fn read_paths(dir: &Path) -> io::Result<(Paths, u64)> {
    let Some(bytes) = cache::read(dir, PATHS_FILE)? else {
        return Ok((Vec::new(), 0));
    };
    let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let paths = bytes[..end]
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice::<String>(l).map(Arc::from))
        .collect::<Result<Paths, _>>()
        .map_err(io::Error::other)?;
    Ok((paths, end as u64))
}

fn to_record(row: Row, paths: &Paths) -> Option<CommitRecord> {
    let get = |i: u32| paths.get(i as usize).cloned();
    let files = row
        .files
        .into_iter()
        .map(|f| {
            Some(FileStat {
                path: get(f.path)?,
                additions: f.additions,
                deletions: f.deletions,
                lines: f.lines,
                rewritten: f.rewritten,
                code: f.code,
                old: match f.old {
                    Some(i) => Some(get(i)?),
                    None => None,
                },
                binary: f.binary,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(CommitRecord {
        id: row.id,
        author: row.author,
        email: row.email,
        time: row.time,
        offset: row.offset,
        merge: row.merge,
        parents: row.parents,
        kind: row.kind,
        scope: row.scope,
        files,
    })
}

/// Flush interval: a crash loses at most this many records of progress.
const FLUSH_EVERY: usize = 256;

/// Why a cache cannot be written by this build.
pub(crate) fn newer_message(found: &str) -> String {
    format!(
        "the cache in this repository has format {found}, newer than this build's {FORMAT_VERSION}; \
         leaving it untouched. Run the newer gitterrace, or delete .git/gitterrace to rebuild"
    )
}

/// The stored format when it is newer than ours. Older, missing and unreadable versions are
/// ours to reset; a newer one belongs to a newer build and must never be overwritten.
fn newer_version(dir: &Path) -> io::Result<Option<String>> {
    let found = cache::read(dir, VERSION_FILE)?
        .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
        .unwrap_or_default();
    let newer = match (found.parse::<u64>(), FORMAT_VERSION.parse::<u64>()) {
        (Ok(f), Ok(ours)) => f > ours,
        _ => false,
    };
    Ok(newer.then_some(found))
}

/// Fails when the cache now belongs to a newer build. Called right before every write, so a
/// long-running older process cannot append rows in its format to a rewritten cache.
pub(crate) fn check_writable(dir: &Path) -> io::Result<()> {
    newer_version(dir)?.map_or(Ok(()), |found| Err(io::Error::other(newer_message(&found))))
}

/// Creates `dir` and resets it when its format version differs from ours. A newer version is
/// left alone and returned, so the caller can refuse to use it.
#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
)]
pub(crate) fn ensure_dir(dir: &Path) -> io::Result<Option<String>> {
    cache::cache_dir(dir)?;
    if let Some(found) = newer_version(dir)? {
        eprintln!("gitterrace: {}", newer_message(&found));
        return Ok(Some(found));
    }
    if needs_reset(dir)? {
        // Another process may be appending: deleting its files under it would tear them. The
        // check is repeated under the lock, since the writer that held it may have reset too.
        // Only a reset takes the lock, so a read-only cache that is current stays readable.
        let _lock = WriteLock::take(dir)?;
        if version_stale(dir)?
            || (cache::len(dir, COMMITS_FILE) > 0 && cache::len(dir, PATHS_FILE) == 0)
            || manifest_unreadable(dir)
        {
            wipe(dir)?;
        } else {
            // A crash's temp file or a retired file: not part of the live set, so it goes alone.
            sweep(dir, &known_files(dir))?;
        }
    }
    Ok(None)
}

/// Deletes every file in the cache except the lock and writes our `VERSION`. A cache this build
/// cannot read is never partly used: whatever it held, old formats, stray or half-written files,
/// goes. The caller holds the lock and has checked the cache is not a newer build's.
pub(crate) fn wipe(dir: &Path) -> io::Result<()> {
    sweep(dir, &[LOCK_FILE.to_owned()])?;
    cache::write_atomic(dir, VERSION_FILE, format!("{FORMAT_VERSION}\n").as_bytes())
}

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
        self.builder
            .finish(&mut |sec, items, payload| w.block(sec, items, payload))?;
        self.rows
            .finish(&mut |sec, items, payload| w.block(sec, items, payload))?;
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
    let mut rows = crate::feed::RowWriter::new(dir, g)?;
    for r in records {
        let rank = u32::try_from(oids.len()).map_err(|_| too_many())?;
        fold(&mut strings, &mut labels, &mut builder, &mut rows, r, rank)?;
        oids.push(parse_oid(&r.id)?);
    }
    let seg_name = format!("seg-{g}-{n}.gts");
    let mut w = ContainerWriter::new(dir, &seg_name, Kind::Segment, base)?;
    builder.finish(&mut |sec, items, payload| w.block(sec, items, payload))?;
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
    let plain: Vec<_> = previous
        .live
        .iter()
        .filter(|f| f.kind == 0)
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

/// Wipes the cache when the clone's shallow boundary is not the one its segments were built
/// against: deepening or grafting changes parents, so topo ranks and the order are stale.
/// Treated exactly as an unreadable cache, never partly served. Call before an update, under no
/// lock. A newer build's cache is left alone. Returns whether it wiped.
pub(crate) fn check_shallow(dir: &Path, shallow: &[String]) -> io::Result<bool> {
    let stale = |dir: &Path| -> bool {
        match crate::seg::manifest::Manifest::load(dir) {
            Ok(Some(m)) => shallow_of(shallow).map_or(true, |s| s != m.shallow),
            _ => false,
        }
    };
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

/// Removes every entry of `dir` not named in `live`, the retired-file sweep. Links and
/// special files go through `cache::remove`, which never follows them. A file that cannot be
/// removed now (open elsewhere, on Windows) is left for the next sweep, so this never fails on
/// it. Directories are left alone: the cache never makes one.
#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
)]
pub(crate) fn sweep(dir: &Path, live: &[String]) -> io::Result<()> {
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !live.iter().any(|l| l == name) && !in_flight(dir, name) {
            names.push(name.to_owned());
        }
    }
    names.sort();
    for name in names {
        if std::fs::symlink_metadata(dir.join(&name)).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        if let Err(e) = cache::remove(dir, &name) {
            eprintln!("gitterrace: could not remove {name} from the cache, retrying later: {e}");
        }
    }
    Ok(())
}

/// Whether the stored format differs from ours.
fn version_stale(dir: &Path) -> io::Result<bool> {
    let current = cache::read(dir, VERSION_FILE)?;
    let current = current
        .as_deref()
        .map(|b| String::from_utf8_lossy(b).trim().to_owned());
    Ok(current.as_deref() != Some(FORMAT_VERSION))
}

/// Whether `ensure_dir` has files to delete or sweep: a stale format, records without paths, or
/// a file this format does not name.
fn needs_reset(dir: &Path) -> io::Result<bool> {
    Ok(version_stale(dir)?
        || (cache::len(dir, COMMITS_FILE) > 0 && cache::len(dir, PATHS_FILE) == 0)
        || manifest_unreadable(dir)
        || has_foreign(dir)?)
}

/// Files of this format: the documented registry plus `MANIFEST` and every file it names, so
/// a newly registered file is live without a second list to forget. Anything else is another
/// format's or a crash's leftover. A manifest that does not decode names nothing: its files
/// are then strays and the manifest itself is not live, so the cache is wiped and rebuilt.
fn known_files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = crate::CACHE_FILES
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    if let Ok(Some(m)) = crate::seg::manifest::Manifest::load(dir) {
        out.push(crate::seg::manifest::MANIFEST.to_owned());
        out.extend(m.live.into_iter().map(|f| f.name));
    }
    out
}

/// A temp file younger than this may be another process's write in progress: `lines.json`
/// (insights/headfiles.rs) is written outside the lock, so its temp file can appear while a
/// writer sweeps under the lock. An older one is a crash's leftover and goes.
const TEMP_GRACE: std::time::Duration = std::time::Duration::from_secs(3600);

fn in_flight(dir: &Path, name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tmp"))
        && std::fs::symlink_metadata(dir.join(name))
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().map_or(true, |age| age < TEMP_GRACE))
}

/// A `MANIFEST` that does not decode or names a file that is missing or differs from it: the
/// set it describes cannot be served whole, so the cache is wiped and rebuilt.
fn manifest_unreadable(dir: &Path) -> bool {
    match crate::seg::manifest::Manifest::load(dir) {
        Ok(None) => false,
        Ok(Some(m)) => m.verify(dir).is_err(),
        Err(_) => true,
    }
}

fn has_foreign(dir: &Path) -> io::Result<bool> {
    let known = known_files(dir);
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        if !name
            .to_str()
            .is_some_and(|n| known.iter().any(|k| k == n) || in_flight(dir, n))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Fails instead of misreading a cache a newer build wrote.
fn check_readable(dir: &Path) -> io::Result<()> {
    check_writable(dir)
}

/// Reads all complete lines. A last line without a trailing newline is torn and dropped, as is
/// any line that does not parse. Returns the records and the byte length of the valid prefix.
#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
)]
pub(crate) fn read_records(dir: &Path, threads: usize) -> io::Result<(Vec<CommitRecord>, u64)> {
    let Some(bytes) = cache::read(dir, COMMITS_FILE)? else {
        return Ok((Vec::new(), 0));
    };
    let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    check_readable(dir)?;
    let (paths, _) = read_paths(dir)?;
    let records = parse_parallel(&bytes[..end], threads, &paths);
    let lines = bytes[..end]
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .count();
    if records.len() < lines {
        eprintln!(
            "gitterrace: {} of {lines} cached records could not be read and are ignored",
            lines - records.len()
        );
    }
    Ok((records, end as u64))
}

fn parse_lines(chunk: &[u8], paths: &Paths) -> Vec<CommitRecord> {
    chunk
        .split(|&b| b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Row>(line).ok())
        .filter_map(|row| to_record(row, paths))
        .collect()
}

/// Below this a single thread is faster than spawning.
const PARALLEL_MIN: usize = 256 * 1024;

/// Threads used when none are configured: one per core, at most 8.
fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .min(8)
}

/// Splits `data` (whole lines) at newline boundaries into at most `threads` chunks.
fn split_lines(data: &[u8], threads: usize) -> Vec<&[u8]> {
    let mut chunks: Vec<&[u8]> = Vec::with_capacity(threads);
    let mut start = 0;
    for i in 1..threads {
        let target = (data.len() * i / threads).max(start);
        let cut = data[target..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(data.len(), |p| target + p + 1);
        if cut > start {
            chunks.push(&data[start..cut]);
            start = cut;
        }
    }
    if start < data.len() {
        chunks.push(&data[start..]);
    }
    chunks
}

/// Parses the chunks on scoped threads and joins them in file order. `threads` of 0 means the
/// default.
fn parse_parallel(data: &[u8], threads: usize, paths: &Paths) -> Vec<CommitRecord> {
    let threads = if threads == 0 {
        default_threads()
    } else {
        threads
    };
    if threads < 2 || data.len() < PARALLEL_MIN {
        return parse_lines(data, paths);
    }
    let chunks = split_lines(data, threads);
    std::thread::scope(|s| {
        #[expect(
            clippy::needless_collect,
            reason = "all threads must be spawned before the first join, or they would run one after another"
        )]
        let handles: Vec<_> = chunks
            .iter()
            .map(|c| s.spawn(move || parse_lines(c, paths)))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

/// Byte length of the cache file, a cheap generation marker for callers that cache records.
pub(crate) fn file_len(dir: &Path) -> u64 {
    cache::len(dir, COMMITS_FILE)
}

const ID_PREFIX: &[u8] = b"{\"id\":\"";
const SHALLOW_MARK: &[u8] = b"\",\"shallow\":true";

/// What the commits file already holds, read from each line's prefix without parsing it.
#[derive(Default)]
pub(crate) struct Stored {
    pub(crate) ids: HashSet<String>,
    /// Ids whose newest row was diffed at a shallow boundary.
    pub(crate) shallow: HashSet<String>,
    /// Byte length of the valid prefix: a torn last line is cut there.
    pub(crate) valid: u64,
}

impl Stored {
    /// Of `ids`, those not stored, plus those stored as a shallow boundary that the clone no
    /// longer has (`still_shallow` lists the current boundary): they need a fresh row.
    pub(crate) fn todo<'a>(
        &self,
        ids: &'a [String],
        still_shallow: &HashSet<String>,
    ) -> Vec<&'a String> {
        let mut seen = HashSet::new();
        ids.iter()
            .filter(|id| {
                let fresh = !self.ids.contains(*id)
                    || (self.shallow.contains(*id) && !still_shallow.contains(*id));
                fresh && seen.insert(id.as_str())
            })
            .collect()
    }
}

/// Ids already stored, which of them are shallow-boundary rows, and the valid prefix length.
/// A line that is not a complete object is skipped as torn.
pub(crate) fn stored_ids(dir: &Path) -> io::Result<Stored> {
    check_readable(dir)?;
    let Some(bytes) = cache::read(dir, COMMITS_FILE)? else {
        return Ok(Stored::default());
    };
    let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let mut out = Stored {
        valid: end as u64,
        ..Stored::default()
    };
    for l in bytes[..end].split(|&b| b == b'\n') {
        if !(l.ends_with(b"}") && l.starts_with(ID_PREFIX)) {
            continue;
        }
        let rest = &l[ID_PREFIX.len()..];
        let Some(n) = rest.iter().position(|&b| b == b'"') else {
            continue;
        };
        let Ok(id) = std::str::from_utf8(&rest[..n]) else {
            continue;
        };
        // Rows for one id are read in file order and the last one counts, as in `select`.
        if rest[n..].starts_with(SHALLOW_MARK) {
            out.shallow.insert(id.to_owned());
        } else {
            out.shallow.remove(id);
        }
        out.ids.insert(id.to_owned());
    }
    Ok(out)
}

/// Holds the cache's writer lock until dropped. Every writer, in this process or another
/// (`serve` beside `mcp`), takes it before reading what is stored and before the appender cuts
/// a torn tail, so two writers never hand out the same path id or cut each other's rows.
pub(crate) struct WriteLock(File);

impl WriteLock {
    /// Blocks until no other writer holds the lock.
    pub(crate) fn take(dir: &Path) -> io::Result<Self> {
        let file = cache::open_append(dir, LOCK_FILE)?;
        file.lock()?;
        Ok(Self(file))
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        // Closing the handle releases the lock too; unlocking first only makes it prompt.
        drop(self.0.unlock());
    }
}

/// Appends records, first cutting any torn tail at `valid` bytes. New paths reach the paths
/// file before the records that refer to them, so a crash never leaves a record pointing at a
/// path that was not stored.
pub(crate) struct Appender {
    dir: std::path::PathBuf,
    records: File,
    paths_file: File,
    ids: HashMap<Arc<str>, u32>,
    count: u32,
    new_paths: Vec<u8>,
    buf: Vec<u8>,
    pending: usize,
}

impl Appender {
    pub(crate) fn open(dir: &Path, valid: u64) -> io::Result<Self> {
        check_writable(dir)?;
        let records = cache::open_append(dir, COMMITS_FILE)?;
        if records.metadata()?.len() > valid {
            records.set_len(valid)?;
        }
        let (paths, paths_valid) = read_paths(dir)?;
        let paths_file = cache::open_append(dir, PATHS_FILE)?;
        if paths_file.metadata()?.len() > paths_valid {
            paths_file.set_len(paths_valid)?;
        }
        let count = u32::try_from(paths.len()).map_err(io::Error::other)?;
        let ids = paths.into_iter().zip(0..).collect();
        Ok(Self {
            dir: dir.to_path_buf(),
            records,
            paths_file,
            ids,
            count,
            new_paths: Vec::new(),
            buf: Vec::new(),
            pending: 0,
        })
    }

    fn intern(&mut self, path: &Arc<str>) -> io::Result<u32> {
        if let Some(&id) = self.ids.get(path) {
            return Ok(id);
        }
        let id = self.count;
        self.count += 1;
        serde_json::to_writer(&mut self.new_paths, &**path).map_err(io::Error::other)?;
        self.new_paths.push(b'\n');
        self.ids.insert(Arc::clone(path), id);
        Ok(id)
    }

    /// Appends `record`; `shallow` marks a commit diffed at a shallow boundary.
    pub(crate) fn push(&mut self, record: &CommitRecord, shallow: bool) -> io::Result<()> {
        let mut files = Vec::with_capacity(record.files.len());
        for f in &record.files {
            files.push(FileRow {
                path: self.intern(&f.path)?,
                additions: f.additions,
                deletions: f.deletions,
                lines: f.lines,
                rewritten: f.rewritten,
                code: f.code,
                old: match &f.old {
                    Some(o) => Some(self.intern(o)?),
                    None => None,
                },
                binary: f.binary,
            });
        }
        let row = Row {
            id: record.id.clone(),
            shallow,
            author: record.author.clone(),
            email: record.email.clone(),
            time: record.time,
            offset: record.offset,
            merge: record.merge,
            parents: record.parents.clone(),
            kind: record.kind.clone(),
            scope: record.scope.clone(),
            files,
        };
        serde_json::to_writer(&mut self.buf, &row).map_err(io::Error::other)?;
        self.buf.push(b'\n');
        self.pending += 1;
        if self.pending >= FLUSH_EVERY {
            self.flush()?;
        }
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.pending = 0;
        check_writable(&self.dir)?;
        self.paths_file.write_all(&self.new_paths)?;
        self.new_paths.clear();
        self.records.write_all(&self.buf)?;
        self.buf.clear();
        Ok(())
    }
}

/// Stored records in `order`, skipping ids not stored. Of several rows for one id the last
/// wins: a shallow-boundary row is superseded by the one rebuilt after an unshallow.
pub(crate) fn select(records: Vec<CommitRecord>, order: &[String]) -> Vec<CommitRecord> {
    let mut by_id: HashMap<String, CommitRecord> = HashMap::with_capacity(records.len());
    for r in records {
        by_id.insert(r.id.clone(), r);
    }
    order.iter().filter_map(|id| by_id.remove(id)).collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;

    use crate::seg::format::section;

    use super::{
        COMMITS_FILE, FORMAT_VERSION, LOCK_FILE, PATHS_FILE, VERSION_FILE, ensure_dir, split_lines,
    };

    fn names(dir: &std::path::Path) -> io::Result<Vec<String>> {
        let mut out: Vec<String> = fs::read_dir(dir)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<io::Result<_>>()?;
        out.sort();
        Ok(out)
    }

    /// An older format deletes everything it held, not only the files this build knows.
    #[test]
    fn an_unreadable_cache_is_deleted_whole() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("gitterrace");
        fs::create_dir(&dir)?;
        fs::write(dir.join(VERSION_FILE), "3\n")?;
        fs::write(dir.join(COMMITS_FILE), "{}\n")?;
        fs::write(dir.join(PATHS_FILE), "\"a\"\n")?;
        fs::write(dir.join("seg-1-0.gts"), "old segment")?;
        assert_eq!(ensure_dir(&dir)?, None);
        assert_eq!(names(&dir)?, [VERSION_FILE, LOCK_FILE]);
        assert_eq!(
            fs::read_to_string(dir.join(VERSION_FILE))?.trim(),
            FORMAT_VERSION
        );
        Ok(())
    }

    /// A newer build's cache stays byte for byte, stray files included.
    #[test]
    fn a_newer_cache_stays_untouched() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("gitterrace");
        fs::create_dir(&dir)?;
        fs::write(dir.join(VERSION_FILE), "999\n")?;
        fs::write(dir.join("seg-9-0.gts"), "future")?;
        assert_eq!(ensure_dir(&dir)?.as_deref(), Some("999"));
        assert_eq!(names(&dir)?, ["VERSION", "seg-9-0.gts"]);
        Ok(())
    }

    /// A built segment set survives reopening whole. One flipped byte in a live file wipes the
    /// whole cache, so the set is never partly served.
    #[test]
    fn a_built_segment_set_is_kept_and_a_corrupt_one_wiped() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("gitterrace");
        assert_eq!(ensure_dir(&dir)?, None);
        let mut b = super::SegmentBuild::new(&dir, None, &[]).map_err(io::Error::other)?;
        for (i, path) in ["a.rs", "b.rs", "a.rs"].into_iter().enumerate() {
            let r = crate::CommitRecord {
                id: format!("{i:040x}"),
                author: "Ann".into(),
                email: "ann@x".into(),
                time: 1_700_000_000 + i64::try_from(i).unwrap_or(0) * 86_400,
                offset: 0,
                merge: false,
                parents: Vec::new(),
                kind: Some("feat".into()),
                scope: None,
                files: vec![crate::FileStat {
                    path: path.into(),
                    additions: 1,
                    deletions: 0,
                    lines: 1,
                    rewritten: 0,
                    code: None,
                    old: None,
                    binary: false,
                }],
            };
            b.push(&r).map_err(io::Error::other)?;
        }
        let m = b.finish(None, Vec::new()).map_err(io::Error::other)?;
        let live: Vec<&str> = m.live.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(live, ["seg-1-0.gts", "str-1-0.gts", "ord-1.gts"]);
        assert_eq!(m.commits, 3);
        assert_eq!(ensure_dir(&dir)?, None);
        assert_eq!(
            names(&dir)?,
            [
                "MANIFEST",
                VERSION_FILE,
                LOCK_FILE,
                "ord-1.gts",
                "seg-1-0.gts",
                "str-1-0.gts"
            ]
        );
        let seg = dir.join("seg-1-0.gts");
        let mut bytes = fs::read(&seg)?;
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&seg, bytes)?;
        assert_eq!(ensure_dir(&dir)?, None);
        assert_eq!(names(&dir)?, [VERSION_FILE, LOCK_FILE]);
        Ok(())
    }

    /// A deepened (or newly shallow) clone changes topo ranks: the shallow fingerprint in the
    /// manifest then differs and the cache is wiped like an unreadable one.
    #[test]
    fn a_changed_shallow_set_rebuilds() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("gitterrace");
        assert_eq!(ensure_dir(&dir)?, None);
        let boundary = vec![format!("{:040x}", 7)];
        let b = super::SegmentBuild::new(&dir, None, &boundary).map_err(io::Error::other)?;
        let m = b.finish(None, Vec::new()).map_err(io::Error::other)?;
        assert_ne!(m.shallow, 0);
        assert!(!super::check_shallow(&dir, &boundary)?);
        assert!(names(&dir)?.contains(&"MANIFEST".to_owned()));
        assert!(super::check_shallow(&dir, &[])?);
        assert_eq!(names(&dir)?, [VERSION_FILE, LOCK_FILE]);
        Ok(())
    }

    fn rec(i: usize, path: &str) -> crate::CommitRecord {
        crate::CommitRecord {
            id: format!("{i:040x}"),
            author: if i.is_multiple_of(2) { "Ann" } else { "Bob" }.into(),
            email: "x@x".into(),
            time: 1_700_000_000 + i64::try_from(i).unwrap_or(0) * 86_400,
            offset: 0,
            merge: false,
            parents: Vec::new(),
            kind: Some("fix".into()),
            scope: Some(format!("s{}", i % 3)),
            files: vec![crate::FileStat {
                path: path.into(),
                additions: 2,
                deletions: 1,
                lines: 3,
                rewritten: 0,
                code: None,
                old: None,
                binary: false,
            }],
        }
    }

    fn build(dir: &std::path::Path, recs: &[crate::CommitRecord]) -> io::Result<Vec<Vec<u8>>> {
        let prev = crate::seg::manifest::Manifest::load(dir).map_err(io::Error::other)?;
        let mut b = super::SegmentBuild::new(dir, prev.as_ref(), &[]).map_err(io::Error::other)?;
        for r in recs {
            b.push(r).map_err(io::Error::other)?;
        }
        let m = b.finish(prev, Vec::new()).map_err(io::Error::other)?;
        m.live.iter().map(|f| fs::read(dir.join(&f.name))).collect()
    }

    /// Twenty small updates, each appended as a segment, merged in the background whenever the
    /// set holds more than eight. Right after every merge the bytes equal a one-step build of
    /// the same commits. Some updates bring commits dated before ones already stored (a merged
    /// branch), and some are merge commits, so the merge must renumber into canonical order.
    #[test]
    fn twenty_updates_with_background_merges_equal_one_step() -> io::Result<()> {
        let mut all: Vec<crate::CommitRecord> = Vec::new();
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("gitterrace");
        assert_eq!(ensure_dir(&dir)?, None);
        let mut merges = 0;
        for u in 0..20usize {
            // Two commits per update. Every fourth update's second commit is dated before
            // everything stored so far, every fifth update's first commit is a merge.
            let mut batch = vec![rec(100 + 2 * u, &format!("p{}", u % 5))];
            let mut old = rec(2 * u + 1, &format!("q{u}"));
            if u % 4 == 3 {
                old.time = 1_600_000_000 + i64::try_from(u).unwrap_or(0);
            }
            batch.push(old);
            if u % 5 == 4 {
                let extra = crate::FileStat {
                    path: format!("m{u}").into(),
                    ..batch[0].files[0].clone()
                };
                batch[0].merge = true;
                batch[0].files.push(extra);
            }
            all.extend(batch.iter().cloned());
            match crate::seg::manifest::Manifest::load(&dir).map_err(io::Error::other)? {
                None => {
                    let mut b =
                        super::SegmentBuild::new(&dir, None, &[]).map_err(io::Error::other)?;
                    for r in &batch {
                        b.push(r).map_err(io::Error::other)?;
                    }
                    b.finish(None, Vec::new()).map_err(io::Error::other)?;
                }
                Some(prev) => {
                    super::append_segment(&dir, prev, &[], &batch).map_err(io::Error::other)?;
                }
            }
            // No parents in this fixture, so the canonical order is (time, oid).
            let mut canonical = all.clone();
            canonical.sort_by(|a, b| (a.time, &a.id).cmp(&(b.time, &b.id)));
            let ids: Vec<String> = canonical.iter().map(|r| r.id.clone()).collect();
            let by_id: std::collections::HashMap<String, crate::CommitRecord> =
                all.iter().map(|r| (r.id.clone(), r.clone())).collect();
            let mut fetch = |id: &str| {
                by_id
                    .get(id)
                    .cloned()
                    .ok_or_else(|| io::Error::other("missing record"))
            };
            if super::merge_if_needed(&dir, &ids, &[], Vec::new(), &mut fetch)? {
                merges += 1;
                let one = tempfile::tempdir()?;
                let want = build(one.path(), &canonical)?;
                let m = crate::seg::manifest::Manifest::load(&dir)
                    .map_err(io::Error::other)?
                    .ok_or_else(|| io::Error::other("no manifest"))?;
                let got: Vec<Vec<u8>> = m
                    .live
                    .iter()
                    .map(|f| fs::read(dir.join(&f.name)))
                    .collect::<io::Result<_>>()?;
                assert_eq!(got, want, "after update {u}");
                assert_eq!(m.live.len(), 3, "one segment, one delta, one order file");
            }
            // The set itself must stay readable whole between merges.
            crate::seg::read::Store::open(&dir).map_err(io::Error::other)?;
            let segs = names(&dir)?
                .iter()
                .filter(|n| n.starts_with("seg-"))
                .count();
            assert!(
                segs <= super::MAX_SEGMENTS,
                "{segs} segments after update {u}"
            );
        }
        assert_eq!(merges, 2);
        Ok(())
    }

    /// Two-step (HEAD~k, then HEAD) followed by the merge equals a one-step build byte for
    /// byte: the merge renumbers by rebuilding from the records in canonical order, so ids and
    /// ordinals cannot depend on the build history. Retired files of the steps are gone.
    #[test]
    fn two_steps_then_merge_equal_one_step() -> io::Result<()> {
        let recs: Vec<_> = ["a", "b", "c", "a", "d", "b", "e"]
            .iter()
            .enumerate()
            .map(|(i, p)| rec(i, p))
            .collect();
        let one = tempfile::tempdir()?;
        let want = build(one.path(), &recs)?;
        let two = tempfile::tempdir()?;
        build(two.path(), &recs[..3])?;
        build(two.path(), &recs[3..])?;
        let got = build(two.path(), &recs)?;
        assert_eq!(got, want);
        // The merge rewrites the commit rows too, one row per commit, not only the tables.
        let store = crate::seg::read::Store::open(two.path())
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("no set"))?;
        let seg = &store.segments[0];
        assert!(!seg.blocks(section::ROWS).is_empty(), "no row section");
        for sec in [section::ROW_INDEX, section::OID_INDEX] {
            let n: u32 = seg.blocks(sec).iter().map(|e| e.items).sum();
            assert_eq!(n as usize, recs.len(), "section {sec}");
        }
        assert!(
            !names(two.path())?
                .iter()
                .any(|n| n.starts_with("seg-1") || n.starts_with("seg-2"))
        );
        Ok(())
    }

    /// A current cache keeps its live files, loses a retired or stray one, and keeps a temp file
    /// that may be another process's write in progress.
    #[test]
    fn the_sweep_removes_only_files_not_live() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("gitterrace");
        fs::create_dir(&dir)?;
        fs::write(dir.join(VERSION_FILE), format!("{FORMAT_VERSION}\n"))?;
        fs::write(dir.join(COMMITS_FILE), "")?;
        fs::write(dir.join("retired-seg.gts"), "x")?;
        fs::write(dir.join(".lines.json.1.2.tmp"), "x")?;
        assert_eq!(ensure_dir(&dir)?, None);
        assert_eq!(
            names(&dir)?,
            [".lines.json.1.2.tmp", VERSION_FILE, COMMITS_FILE, LOCK_FILE]
        );
        Ok(())
    }

    #[test]
    fn the_thread_setting_bounds_the_chunk_count() {
        let data: Vec<u8> = (0..1000)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();
        for n in [2, 3, 5] {
            let chunks = split_lines(&data, n);
            assert!(
                chunks.len() <= n && chunks.len() > 1,
                "{n} threads: {}",
                chunks.len()
            );
            assert_eq!(chunks.concat(), data, "no line is lost or cut");
        }
        assert_eq!(split_lines(&data, 1).len(), 1);
    }
}
