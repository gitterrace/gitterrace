mod segments;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use crate::{CodeStat, CommitRecord, FileStat, cache};

pub(crate) use segments::{
    MAX_SEGMENTS, SegmentBuild, append_segment, check_shallow, load_order, merge_if_needed, oid_of,
};

/// Cache format; a different one on disk discards the records.
pub const FORMAT_VERSION: &str = "5";
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
mod tests;
