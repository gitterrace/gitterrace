//! Ignore rules (`.gitignore` and `.ignore`) as committed at a revision. Leaves build
//! output and vendored paths out of statistics even when old commits tracked them.

use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;
use gix::glob::pattern::Case;
use gix::ignore::{Search, search::Ignore};

use crate::{BlobReader, Repo, Result, files};

/// Largest ignore or mailmap file read: these are configuration, never megabytes.
pub(crate) const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const FILE_NAMES: [&str; 2] = [".gitignore", ".ignore"];

/// All ignore files of one revision, deeper files overriding shallower ones.
pub struct IgnoreSet {
    search: Search,
}

impl IgnoreSet {
    /// Reads every `.gitignore` and `.ignore` blob in the tree of `rev`, never the worktree.
    pub fn at(repo: &Repo, rev: &str) -> Result<Self> {
        let reader = BlobReader::open(repo)?;
        let mut found: Vec<(String, String)> = files(repo, rev)?
            .into_iter()
            .filter(|f| {
                let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                FILE_NAMES.contains(&name)
            })
            .filter_map(|f| {
                reader
                    .text(f.blob, MAX_CONFIG_BYTES)
                    .map(|text| (f.path, text))
            })
            .collect();
        // Shallow first: the search tries the last added list first, so deeper rules win.
        found.sort_by_key(|(path, _)| path.matches('/').count());
        Ok(Self::from_files(found))
    }

    /// Builds the set from `(path of the ignore file, its text)` pairs, shallow files first.
    pub fn from_files(found: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut search = Search::default();
        for (path, text) in found {
            // An unreadable list only means fewer rules.
            let added = search.add_patterns_buffer(
                text.as_bytes(),
                PathBuf::from(&path),
                Some(Path::new("")),
                Ignore::default(),
            );
            drop(added);
        }
        Self { search }
    }

    /// True when `path` (repository relative, `/` separated) or one of its parent directories
    /// is ignored.
    pub fn is_ignored(&self, path: &str) -> bool {
        let mut end = 0;
        while let Some(i) = path[end..].find('/') {
            end += i;
            if self.hit(&path[..end], true) {
                return true;
            }
            end += 1;
        }
        self.hit(path, false)
    }

    fn hit(&self, path: &str, is_dir: bool) -> bool {
        self.search
            .pattern_matching_relative_path(
                path.as_bytes().as_bstr(),
                Some(is_dir),
                Case::Sensitive,
            )
            .is_some_and(|m| !m.pattern.is_negative())
    }
}

#[cfg(test)]
mod tests {
    use super::IgnoreSet;

    fn set(files: &[(&str, &str)]) -> IgnoreSet {
        IgnoreSet::from_files(
            files
                .iter()
                .map(|(p, t)| ((*p).to_owned(), (*t).to_owned())),
        )
    }

    #[test]
    fn root_rules_dirs_and_negation() {
        let s = set(&[(".gitignore", "target/\n*.log\n!keep.log\n/only-root.txt\n")]);
        assert!(s.is_ignored("target/debug/x.rs"));
        assert!(s.is_ignored("a/b/target/x"));
        assert!(s.is_ignored("run.log"));
        assert!(!s.is_ignored("keep.log"));
        assert!(s.is_ignored("only-root.txt"));
        assert!(!s.is_ignored("sub/only-root.txt"));
        assert!(!s.is_ignored("src/main.rs"));
    }

    #[test]
    fn nested_files_are_scoped_and_override() {
        let s = set(&[
            (".gitignore", "*.gen\n"),
            ("web/.gitignore", "dist/\n!a.gen\n"),
        ]);
        assert!(s.is_ignored("web/dist/app.js"));
        assert!(!s.is_ignored("dist/app.js"));
        assert!(s.is_ignored("x/b.gen"));
        assert!(!s.is_ignored("web/a.gen"));
    }
}
