use std::collections::HashMap;
use std::io;
use std::path::Path;

use gitterrace_git::{BlobReader, MAX_TEXT_BYTES, Repo};

use super::{Insights, PathFilter, language};

pub(crate) const CACHE_FILE: &str = "lines.json";

/// A text file of the head tree with its line count.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HeadFile {
    pub path: String,
    pub lines: u32,
}

/// Language share of the head tree.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LangShare {
    pub language: String,
    pub lines: u64,
    pub files: usize,
    pub share: f64,
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

/// Share per language of `files`, largest first, skipping what `filter` skips.
pub fn language_share(files: &[HeadFile], filter: &PathFilter) -> Vec<LangShare> {
    let mut map: HashMap<String, (u64, usize)> = HashMap::new();
    for f in files.iter().filter(|f| !filter.skips(&f.path)) {
        let e = map.entry(language(&f.path)).or_default();
        e.0 += u64::from(f.lines);
        e.1 += 1;
    }
    let total: u64 = map.values().map(|v| v.0).sum();
    let mut out: Vec<LangShare> = map
        .into_iter()
        .map(|(language, (lines, files))| LangShare {
            language,
            lines,
            files,
            share: if total == 0 {
                0.0
            } else {
                lines as f64 / total as f64
            },
        })
        .collect();
    out.sort_by(|a, b| {
        b.lines
            .cmp(&a.lines)
            .then_with(|| a.language.cmp(&b.language))
    });
    out
}

impl Insights<'_> {
    /// [`language_share`] under this instance's options.
    pub fn languages_now(&self, files: &[HeadFile]) -> Vec<LangShare> {
        language_share(files, &self.filter)
    }
}

#[cfg(test)]
mod tests {
    use super::{HeadFile, LineCache, language_share};
    use crate::{Options, PathFilter};

    #[test]
    fn share_sums_and_skips() {
        let files = [
            ("a.rs", 30),
            ("b.rs", 50),
            ("c.md", 20),
            ("Cargo.lock", 900),
        ]
        .map(|(p, n)| HeadFile {
            path: p.into(),
            lines: n,
        });
        let f = PathFilter::new(&Options::default(), None);
        let s = language_share(&files, &f);
        assert_eq!(
            (s[0].language.as_str(), s[0].lines, s[0].files),
            ("Rust", 80, 2)
        );
        assert!((s[0].share - 0.8).abs() < 1e-9);
        assert_eq!(s.len(), 2);
    }

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
