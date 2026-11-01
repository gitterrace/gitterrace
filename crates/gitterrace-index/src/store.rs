use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use crate::{CodeStat, CommitRecord, FileStat, cache};

/// Cache format; a different one on disk discards the records.
pub const FORMAT_VERSION: &str = "3";
pub(crate) const VERSION_FILE: &str = "VERSION";
pub(crate) const COMMITS_FILE: &str = "commits.jsonl";
pub(crate) const PATHS_FILE: &str = "paths.jsonl";

#[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if signature
fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A record as stored: file paths are numbers into the paths file, so a path that thousands of
/// commits touch is written once. `id` stays the first field, see [`stored_ids`].
#[derive(serde::Serialize, serde::Deserialize)]
struct Row {
    id: String,
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
    match newer_version(dir)? {
        Some(found) => Err(io::Error::other(newer_message(&found))),
        None => Ok(()),
    }
}

/// Creates `dir` and resets it when its format version differs from ours. A newer version is
/// left alone and returned, so the caller can refuse to use it.
pub(crate) fn ensure_dir(dir: &Path) -> io::Result<Option<String>> {
    cache::cache_dir(dir)?;
    if let Some(found) = newer_version(dir)? {
        eprintln!("gitterrace: {}", newer_message(&found));
        return Ok(Some(found));
    }
    let current = cache::read(dir, VERSION_FILE)?;
    let current = current
        .as_deref()
        .map(|b| String::from_utf8_lossy(b).trim().to_owned());
    if current.as_deref() != Some(FORMAT_VERSION) {
        cache::remove(dir, COMMITS_FILE)?;
        cache::remove(dir, PATHS_FILE)?;
        cache::write_atomic(dir, VERSION_FILE, format!("{FORMAT_VERSION}\n").as_bytes())?;
    }
    // Records without their path table cannot be read: start over.
    if cache::len(dir, COMMITS_FILE) > 0 && cache::len(dir, PATHS_FILE) == 0 {
        cache::remove(dir, COMMITS_FILE)?;
    }
    Ok(None)
}

/// Fails instead of misreading a cache a newer build wrote.
fn check_readable(dir: &Path) -> io::Result<()> {
    check_writable(dir)
}

/// Reads all complete lines. A last line without a trailing newline is torn and dropped, as is
/// any line that does not parse. Returns the records and the byte length of the valid prefix.
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

/// Ids already stored, plus the valid prefix length. Reads each id from the line prefix
/// without parsing the line; a line that is not a complete object is skipped as torn.
pub(crate) fn stored_ids(dir: &Path) -> io::Result<(HashSet<String>, u64)> {
    check_readable(dir)?;
    let Some(bytes) = cache::read(dir, COMMITS_FILE)? else {
        return Ok((HashSet::new(), 0));
    };
    let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let ids = bytes[..end]
        .split(|&b| b == b'\n')
        .filter(|l| l.ends_with(b"}") && l.starts_with(ID_PREFIX))
        .filter_map(|l| {
            let rest = &l[ID_PREFIX.len()..];
            let n = rest.iter().position(|&b| b == b'"')?;
            std::str::from_utf8(&rest[..n]).ok().map(str::to_owned)
        })
        .collect();
    Ok((ids, end as u64))
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

    pub(crate) fn push(&mut self, record: &CommitRecord) -> io::Result<()> {
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

/// Stored records in `order`, skipping ids not stored and duplicates (first wins).
pub(crate) fn select(records: Vec<CommitRecord>, order: &[String]) -> Vec<CommitRecord> {
    let mut by_id: HashMap<String, CommitRecord> = HashMap::with_capacity(records.len());
    for r in records {
        by_id.entry(r.id.clone()).or_insert(r);
    }
    order.iter().filter_map(|id| by_id.remove(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::split_lines;

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
