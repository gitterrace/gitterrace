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
    let records = bytes[..end]
        .split(|&b| b == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect();
    Ok((records, end as u64))
}

/// Ids already stored, plus the valid prefix length.
pub(crate) fn stored_ids(dir: &Path) -> io::Result<(HashSet<String>, u64)> {
    let (records, valid) = read_records(dir)?;
    Ok((records.into_iter().map(|r| r.id).collect(), valid))
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
