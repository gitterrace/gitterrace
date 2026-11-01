use std::collections::HashMap;
use std::io;
use std::path::Path;

use gitterrace_git::{BlobReader, MAX_TEXT_BYTES, Repo};

pub(crate) const CACHE_FILE: &str = "lines.json";

/// A text file of the head tree with its line count.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HeadFile {
    pub path: String,
    pub lines: u32,
}

/// Line counts per blob id, kept in `.git/gitterrace/lines.json`. A blob never changes, so the
/// cache never goes stale; losing it only costs a re-read.
#[derive(Debug, Default)]
pub struct LineCache {
    pub(crate) map: HashMap<String, u32>,
    pub(crate) dirty: bool,
}

impl LineCache {
    pub fn load(dir: &Path) -> Self {
        let map = crate::cache::read(dir, CACHE_FILE)
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { map, dirty: false }
    }

    pub fn save(&mut self, dir: &Path) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&self.map).map_err(io::Error::other)?;
        crate::cache::write_atomic(dir, CACHE_FILE, &bytes)?;
        self.dirty = false;
        Ok(())
    }
}

/// Text files of the tree at `rev` with line counts. Binary and oversized blobs are left out.
/// Missing counts are read once and remembered in `cache`.
pub fn head_files(
    repo: &Repo,
    rev: &str,
    cache: &mut LineCache,
) -> gitterrace_git::Result<Vec<HeadFile>> {
    let reader = BlobReader::open(repo)?;
    let mut out = Vec::new();
    for f in gitterrace_git::files(repo, rev)? {
        let key = f.blob.hex();
        let lines = if let Some(n) = cache.map.get(&key) {
            *n
        } else {
            // Binary or oversized blobs are cached as 0 so they are not re-read.
            let n = reader.line_count(f.blob, MAX_TEXT_BYTES).unwrap_or(0);
            cache.map.insert(key, n);
            cache.dirty = true;
            n
        };
        if lines > 0 {
            out.push(HeadFile {
                path: f.path,
                lines,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::LineCache;

    #[test]
    fn cache_round_trips() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut c = LineCache::default();
        c.map.insert("abc".into(), 7);
        c.dirty = true;
        c.save(dir.path())?;
        assert_eq!(LineCache::load(dir.path()).map.len(), 1);
        Ok(())
    }
}
