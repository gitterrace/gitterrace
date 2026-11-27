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
    /// An explicit unmark kept as a tombstone, so a late move of old marks cannot bring the
    /// path back. Never listed by `viewed_list`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub off: bool,
}

impl Index {
    /// Every mark, oldest first.
    ///
    /// # Errors
    /// When the file cannot be read. A file that does not decode reads as no marks.
    pub fn viewed_list(&self) -> io::Result<Vec<Viewed>> {
        Ok(self.viewed_records()?.into_iter().filter(|v| !v.off).collect())
    }

    fn viewed_records(&self) -> io::Result<Vec<Viewed>> {
        let Some(bytes) = cache::read(self.dir(), FILE)? else {
            return Ok(Vec::new());
        };
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    fn viewed_save(&self, mut all: Vec<Viewed>) -> io::Result<()> {
        let cut = all.len().saturating_sub(MAX_MARKS);
        all.drain(..cut);
        let bytes = serde_json::to_vec(&all).map_err(io::Error::other)?;
        cache::write_atomic(self.dir(), FILE, &bytes)
    }

    /// Marks each `(path, patch)` as viewed at `now`, or clears the mark of `path` when `on` is
    /// false. A path has one mark: a new patch hash replaces the old one.
    ///
    /// # Errors
    /// When the cache cannot be written.
    pub fn viewed_set(&self, items: &[(String, String)], on: bool, now: i64) -> io::Result<()> {
        self.writable()?;
        let _lock = crate::store::WriteLock::take(self.dir())?;
        let mut all = self.viewed_records()?;
        for (path, patch) in items {
            if path.len() > MAX_FIELD || patch.len() > MAX_FIELD {
                continue;
            }
            all.retain(|v| v.path != *path);
            all.push(Viewed { path: path.clone(), patch: patch.clone(), at: now, off: !on });
        }
        self.viewed_save(all)
    }

    /// Marks each `(path, patch)` only when the path has no record at all, a mark or an
    /// unmark. The check and the write share one lock, so a newer click of another tab
    /// cannot slip between them. Answers how many were marked.
    ///
    /// # Errors
    /// When the cache cannot be written.
    pub fn viewed_add_missing(&self, items: &[(String, String)], now: i64) -> io::Result<usize> {
        self.writable()?;
        let _lock = crate::store::WriteLock::take(self.dir())?;
        let mut all = self.viewed_records()?;
        let mut held: std::collections::HashSet<String> =
            all.iter().map(|v| v.path.clone()).collect();
        let mut n = 0;
        for (path, patch) in items {
            if path.len() > MAX_FIELD || patch.len() > MAX_FIELD || !held.insert(path.clone()) {
                continue;
            }
            all.push(Viewed { path: path.clone(), patch: patch.clone(), at: now, off: false });
            n += 1;
        }
        if n > 0 {
            self.viewed_save(all)?;
        }
        Ok(n)
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

    #[test]
    fn a_late_move_of_old_marks_keeps_a_newer_unmark_and_a_newer_mark() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let idx = Index::open_dir(tmp.path().join("gitterrace"))?;
        let one = |p: &str, h: &str| vec![(p.to_owned(), h.to_owned())];
        // Another tab ticked then unticked "a" and marked "b" with a newer hash.
        idx.viewed_set(&one("a", "h1"), true, 1)?;
        idx.viewed_set(&one("a", "h1"), false, 2)?;
        idx.viewed_set(&one("b", "new"), true, 3)?;
        let old = vec![
            ("a".to_owned(), "h1".to_owned()),
            ("b".to_owned(), "old".to_owned()),
            ("c".to_owned(), "h3".to_owned()),
        ];
        assert_eq!(idx.viewed_add_missing(&old, 4)?, 1);
        let v: Vec<_> = idx.viewed_list()?.into_iter().map(|m| (m.path, m.patch)).collect();
        assert_eq!(v, [("b".to_owned(), "new".to_owned()), ("c".to_owned(), "h3".to_owned())]);
        // A later explicit mark of "a" replaces the tombstone.
        idx.viewed_set(&one("a", "h9"), true, 5)?;
        assert_eq!(idx.viewed_list()?.len(), 3);
        Ok(())
    }
}
