//! Viewed marks: which file at which patch hash the user has looked at, kept in
//! `.git/gitterrace/viewed`. A mark holds only while the patch hash is the same, so a changed
//! file asks to be looked at again. Small, rewritten whole by atomic rename. Not rebuildable.

use std::io;

use serde::{Deserialize, Serialize};

use crate::{Index, cache};

pub(crate) const FILE: &str = "viewed";

/// Most marks kept, newest win. Not configurable: a bound on a file that grows with use.
const MAX_MARKS: usize = 20_000;
/// Longest path or hash a mark may hold, so a request cannot grow the file by one entry.
const MAX_FIELD: usize = 4096;

/// One mark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewed {
    pub path: String,
    pub patch: String,
    pub at: i64,
}

impl Index {
    /// Every mark, oldest first.
    ///
    /// # Errors
    /// When the file cannot be read. A file that does not decode reads as no marks.
    pub fn viewed_list(&self) -> io::Result<Vec<Viewed>> {
        let Some(bytes) = cache::read(self.dir(), FILE)? else {
            return Ok(Vec::new());
        };
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    /// Marks each `(path, patch)` as viewed at `now`, or clears the mark of `path` when `on` is
    /// false. A path has one mark: a new patch hash replaces the old one.
    ///
    /// # Errors
    /// When the cache cannot be written.
    pub fn viewed_set(&self, items: &[(String, String)], on: bool, now: i64) -> io::Result<()> {
        self.writable()?;
        let _lock = crate::store::WriteLock::take(self.dir())?;
        let mut all = self.viewed_list()?;
        for (path, patch) in items {
            if path.len() > MAX_FIELD || patch.len() > MAX_FIELD {
                continue;
            }
            all.retain(|v| v.path != *path);
            if on {
                all.push(Viewed { path: path.clone(), patch: patch.clone(), at: now });
            }
        }
        let cut = all.len().saturating_sub(MAX_MARKS);
        all.drain(..cut);
        let bytes = serde_json::to_vec(&all).map_err(io::Error::other)?;
        cache::write_atomic(self.dir(), FILE, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_patch_hash_replaces_the_mark_and_off_clears_it() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let idx = Index::open_dir(tmp.path().join("gitterrace"))?;
        let one = |p: &str, h: &str| vec![(p.to_owned(), h.to_owned())];
        idx.viewed_set(&one("a", "h1"), true, 1)?;
        idx.viewed_set(&one("b", "h2"), true, 2)?;
        idx.viewed_set(&one("a", "h3"), true, 3)?;
        let v = idx.viewed_list()?;
        assert_eq!(
            v.iter().map(|m| (m.path.as_str(), m.patch.as_str())).collect::<Vec<_>>(),
            [("b", "h2"), ("a", "h3")]
        );
        idx.viewed_set(&one("b", ""), false, 4)?;
        assert_eq!(idx.viewed_list()?.len(), 1);
        Ok(())
    }

    #[test]
    fn a_file_that_does_not_decode_reads_as_no_marks() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let idx = Index::open_dir(tmp.path().join("gitterrace"))?;
        std::fs::write(idx.dir().join(FILE), "not json")?;
        assert_eq!(idx.viewed_list()?.len(), 0);
        Ok(())
    }
}
