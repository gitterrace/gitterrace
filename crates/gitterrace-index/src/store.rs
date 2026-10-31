use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::CommitRecord;

pub(crate) const FORMAT_VERSION: &str = "2";
pub(crate) const VERSION_FILE: &str = "VERSION";
pub(crate) const COMMITS_FILE: &str = "commits.jsonl";

/// Flush interval: a crash loses at most this many records of progress.
const FLUSH_EVERY: usize = 256;

/// Creates `dir` and resets it when its format version differs from ours.
pub(crate) fn ensure_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let version = dir.join(VERSION_FILE);
    let current = fs::read_to_string(&version).ok();
    if current.as_deref().map(str::trim) != Some(FORMAT_VERSION) {
        match fs::remove_file(dir.join(COMMITS_FILE)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        fs::write(&version, format!("{FORMAT_VERSION}\n"))?;
    }
    Ok(())
}

/// Reads all complete lines. A last line without a trailing newline is torn and dropped, as is
/// any line that does not parse. Returns the records and the byte length of the valid prefix.
pub(crate) fn read_records(dir: &Path) -> io::Result<(Vec<CommitRecord>, u64)> {
    let bytes = match fs::read(dir.join(COMMITS_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => return Err(e),
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
    fs::metadata(dir.join(COMMITS_FILE)).map_or(0, |m| m.len())
}

const ID_PREFIX: &[u8] = b"{\"id\":\"";

/// Ids already stored, plus the valid prefix length. Reads each id from the line prefix
/// without parsing the line; a line that is not a complete object is skipped as torn.
pub(crate) fn stored_ids(dir: &Path) -> io::Result<(HashSet<String>, u64)> {
    let bytes = match fs::read(dir.join(COMMITS_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((HashSet::new(), 0)),
        Err(e) => return Err(e),
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
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(dir.join(COMMITS_FILE))?;
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
