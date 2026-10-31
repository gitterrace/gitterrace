use std::path::Path;

use crate::{BlameLine, Blob, CommitDetail, CommitSummary, RefInfo, Result, TreeEntry};

/// A repository opened read-only.
pub struct Repo {
    inner: gix::ThreadSafeRepository,
}

impl Repo {
    /// Opens the repository containing `path` (discovers upward).
    pub fn discover(path: &Path) -> Result<Self> {
        let inner = gix::ThreadSafeRepository::discover(path)
            .map_err(|e| crate::Error::NotARepo(e.to_string()))?;
        Ok(Self { inner })
    }

    /// Path of the `.git` directory, where gitterrace keeps its cache.
    pub fn git_dir(&self) -> &Path {
        self.inner.git_dir()
    }

    /// Display name: the worktree folder name.
    pub fn name(&self) -> String {
        todo!("gitterrace-git")
    }

    /// Resolves a revision (branch, tag, sha, `HEAD`) to a full commit id.
    pub fn resolve(&self, _rev: &str) -> Result<String> {
        todo!("gitterrace-git")
    }

    /// Commits reachable from `rev`, newest first, optionally limited to `path`.
    pub fn log(
        &self,
        _rev: &str,
        _path: Option<&str>,
        _skip: usize,
        _limit: usize,
    ) -> Result<Vec<CommitSummary>> {
        todo!("gitterrace-git")
    }

    /// Commit ids reachable from `rev`, oldest first. Used by the indexer.
    pub fn rev_list(&self, _rev: &str) -> Result<Vec<String>> {
        todo!("gitterrace-git")
    }

    /// One commit with its diff against the first parent.
    pub fn commit(&self, _id: &str) -> Result<CommitDetail> {
        todo!("gitterrace-git")
    }

    /// Per-file line counts against the first parent, without hunk text.
    /// The indexer calls this once per commit, so it must stay cheap.
    pub fn numstat(&self, _id: &str) -> Result<Vec<crate::FileDiff>> {
        todo!("gitterrace-git")
    }

    /// Entries of the directory at `path` in `rev`, dirs first then by name.
    pub fn tree(&self, _rev: &str, _path: &str) -> Result<Vec<TreeEntry>> {
        todo!("gitterrace-git")
    }

    pub fn blob(&self, _rev: &str, _path: &str) -> Result<Blob> {
        todo!("gitterrace-git")
    }

    pub fn blame(&self, _rev: &str, _path: &str) -> Result<Vec<BlameLine>> {
        todo!("gitterrace-git")
    }

    /// Branches, tags and remote branches, newest first.
    pub fn refs(&self) -> Result<Vec<RefInfo>> {
        todo!("gitterrace-git")
    }

    /// Name of the branch HEAD points at, or `None` when detached.
    pub fn head_branch(&self) -> Option<String> {
        todo!("gitterrace-git")
    }
}
