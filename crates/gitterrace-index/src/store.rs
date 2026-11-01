use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::{CommitRecord, cache};

/// Cache format; a different one on disk discards the records.
pub const FORMAT_VERSION: &str = "2";
pub(crate) const VERSION_FILE: &str = "VERSION";
pub(crate) const COMMITS_FILE: &str = "commits.jsonl";

/// Flush interval: a crash loses at most this many records of progress.
const FLUSH_EVERY: usize = 256;

/// Creates `dir` and resets it when its format version differs from ours.
pub(crate) fn ensure_dir(dir: &Path) -> io::Result<()> {
    cache::cache_dir(dir)?;
    let current = cache::read(dir, VERSION_FILE)?;
    let current = current
        .as_deref()
        .map(|b| String::from_utf8_lossy(b).trim().to_owned());
    if current.as_deref() != Some(FORMAT_VERSION) {
        cache::remove(dir, COMMITS_FILE)?;
        cache::write_atomic(dir, VERSION_FILE, format!("{FORMAT_VERSION}\n").as_bytes())?;
    }
    Ok(())
}

/// Reads all complete lines. A last line without a trailing newline is torn and dropped, as is
/// any line that does not parse. Returns the records and the byte length of the valid prefix.
pub(crate) fn read_records(dir: &Path) -> io::Result<(Vec<CommitRecord>, u64)> {
    let Some(bytes) = cache::read(dir, COMMITS_FILE)? else {
        return Ok((Vec::new(), 0));
    };
    let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    Ok((parse_parallel(&bytes[..end]), end as u64))
}

fn parse_lines(chunk: &[u8]) -> Vec<CommitRecord> {
    chunk
        .split(|&b| b == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect()
}

/// Below this a single thread is faster than spawning.
const PARALLEL_MIN: usize = 256 * 1024;

/// Splits `data` (whole lines) at newline boundaries into one chunk per core, parses the chunks
/// on scoped threads and joins them in file order.
fn parse_parallel(data: &[u8]) -> Vec<CommitRecord> {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    if threads < 2 || data.len() < PARALLEL_MIN {
        return parse_lines(data);
    }
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
    std::thread::scope(|s| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|c| s.spawn(move || parse_lines(c)))
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

/// Appends records, first cutting any torn tail at `valid` bytes.
pub(crate) struct Appender {
    out: BufWriter<File>,
    pending: usize,
}

impl Appender {
    pub(crate) fn open(dir: &Path, valid: u64) -> io::Result<Self> {
        let file = cache::open_append(dir, COMMITS_FILE)?;
        if file.metadata()?.len() > valid {
            file.set_len(valid)?;
        }
        Ok(Self {
            out: BufWriter::new(file),
            pending: 0,
        })
    }

    pub(crate) fn push(&mut self, record: &CommitRecord) -> io::Result<()> {
        serde_json::to_writer(&mut self.out, record).map_err(io::Error::other)?;
        self.out.write_all(b"\n")?;
        self.pending += 1;
        if self.pending >= FLUSH_EVERY {
            self.flush()?;
        }
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.pending = 0;
        self.out.flush()
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
