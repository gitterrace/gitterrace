mod blame;
mod changes;
mod history;
mod objects;

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use gix::bstr::ByteSlice;

use crate::error::git;
use crate::repo::blame::{BLAME_CACHE_BYTES, BlameCache};
pub use crate::repo::changes::ChangeMemo;
use crate::{Error, MAX_TEXT_BYTES, RefInfo, RefKind, Result};

/// Decoded-object cache; trees and commits are re-read constantly by walks and diffs. One per
/// request thread, so it is multiplied by the jobs running at once: kept small.
const OBJECT_CACHE_BYTES: usize = 16 * 1024 * 1024;
/// Object cache of one index worker thread.
const BLAME_OBJECT_CACHE_BYTES: usize = 8 * 1024 * 1024;
const INDEX_WORKER_CACHE_BYTES: usize = 4 * 1024 * 1024;
/// Delta-base (pack) cache of one index worker thread. gix defaults to up to 96 MB per handle,
/// close to 1 GB over ten workers; at 24 MB git.git builds as fast with 400 MB less peak.
const INDEX_WORKER_PACK_CACHE_BYTES: usize = 24 * 1024 * 1024;
/// How far the index workers may run ahead of the oldest commit not yet handed on. A merge
/// whose rename detection takes minutes (linux has many) holds that oldest slot: with a window
/// of 4096 commits, samples of the linux build showed workers waiting on it for over half of
/// their time. So the count is wide, and the held results are bounded by their estimated
/// bytes instead, which a run of huge merges reaches long before the count.
const NUMSTAT_WINDOW: crate::diff::Window = crate::diff::Window {
    items: 1 << 16,
    bytes: 64 * 1024 * 1024,
};
/// Blob pairs whose line counts one index build remembers (about 100 bytes each with the map's
/// overhead, so 256k pairs is about 25 MB). Past that the memo starts over: slower, never
/// different.
const NUMSTAT_MEMO_PAIRS: usize = 1 << 18;
/// Rev syntax that searches history (`:/text`, `^{/text}`) or reads reflogs (`@{...}`).
/// Every caller passes URL input here, so the refusal lives at the one choke point.
const REJECTED_REV: [&str; 3] = [":/", "^{/", "@{"];

/// A repository opened read-only.
pub struct Repo {
    inner: gix::ThreadSafeRepository,
    blame_cache: Mutex<BlameCache>,
    /// Largest blob read as text by this handle, see [`Repo::with_limits`].
    text_limit: AtomicU64,
    /// Changed files the Uncommitted view lists, see [`Repo::set_worktree_limits`].
    worktree_files: AtomicUsize,
    /// Largest file the Uncommitted view diffs, in bytes.
    worktree_diff_bytes: AtomicU64,
    /// Threads [`Repo::numstat_many`] uses; 0 means one per core.
    workers: usize,
    /// Ahead and behind per (branch tip, base): immutable for a pair of commits.
    pub(crate) divergence: Mutex<HashMap<(gix::ObjectId, gix::ObjectId), (usize, usize)>>,
    /// The root-commit id, see `repo_id.rs`.
    pub(crate) id_cache: OnceLock<Option<String>>,
    /// See [`Repo::committed_only`].
    committed_only: AtomicBool,
    /// The export run's memo of tree diffs, see [`Repo::use_change_memo`]. Weak: the run owns
    /// it, and a server handle (none set) keeps nothing.
    changes: Mutex<Weak<ChangeMemo>>,
    /// Commits read for a summary plus divergences walked, see [`Repo::row_reads`].
    pub(crate) row_reads: AtomicU64,
}

impl Repo {
    /// Opens the repository containing `path` (discovers upward).
    pub fn discover(path: &Path) -> Result<Self> {
        let inner = gix::ThreadSafeRepository::discover(path)
            .map_err(|e| crate::Error::NotARepo(e.to_string()))?;
        Ok(Self {
            inner,
            blame_cache: Mutex::new(BlameCache::with_limit(BLAME_CACHE_BYTES)),
            text_limit: AtomicU64::new(MAX_TEXT_BYTES),
            worktree_files: AtomicUsize::new(crate::worktree::WORKTREE_FILES),
            worktree_diff_bytes: AtomicU64::new(crate::worktree::WORKTREE_DIFF_BYTES),
            workers: 0,
            divergence: Mutex::new(HashMap::new()),
            id_cache: OnceLock::new(),
            committed_only: AtomicBool::new(false),
            changes: Mutex::new(Weak::new()),
            row_reads: AtomicU64::new(0),
        })
    }

    /// Commits read by [`Self::summaries`] plus branches walked by [`Self::branch_status`] since
    /// this handle opened. A page's cost per row, so a test can bound it by the rows shown.
    pub fn row_reads(&self) -> u64 {
        self.row_reads.load(Ordering::Relaxed)
    }

    /// From now on every revision this handle resolves is `HEAD`, a full commit id, or a
    /// branch, tag or remote-tracking ref, optionally followed by `~N` / `^N`. Stashes, other
    /// refs, reflogs and abbreviated ids (which enumerate the object store) stop resolving.
    /// What a public server shows, enforced where every URL rev passes. One way: nothing
    /// turns it off again, so a shared handle cannot be widened.
    pub fn restrict_to_committed(&self) {
        self.committed_only.store(true, Ordering::Relaxed);
    }

    /// Shares commit tree diffs through `memo` while its owner keeps it: a commit page and its
    /// folded files then diff the trees once. A memo whose run has ended is replaced, so a
    /// handle reused for the next export shares that run's memo; a live one stays, and a
    /// second run alongside it shares it.
    pub fn use_change_memo(&self, memo: &Arc<ChangeMemo>) {
        if let Ok(mut held) = self.changes.lock()
            && held.upgrade().is_none()
        {
            *held = Arc::downgrade(memo);
        }
    }

    /// The memo in force, if its owner still holds it.
    pub(crate) fn change_memo(&self) -> Option<Arc<ChangeMemo>> {
        self.changes.lock().ok()?.upgrade()
    }

    /// [`Repo::restrict_to_committed`] as a builder.
    #[must_use]
    pub fn committed_only(self) -> Self {
        self.restrict_to_committed();
        self
    }

    /// Sets the largest blob this handle reads as text (blob views, blame, diffs with hunks)
    /// and the memory the blame cache may keep. The index always uses [`MAX_TEXT_BYTES`], so
    /// its stored numbers do not depend on a display setting.
    #[must_use]
    pub fn with_limits(self, text_bytes: u64, blame_cache_bytes: usize) -> Self {
        self.set_limits(text_bytes, blame_cache_bytes);
        self
    }

    /// [`Repo::with_limits`] on a handle in use. A smaller blame cache drops its oldest entries
    /// at once.
    pub fn set_limits(&self, text_bytes: u64, blame_cache_bytes: usize) {
        self.text_limit.store(text_bytes, Ordering::Relaxed);
        if let Ok(mut c) = self.blame_cache.lock() {
            c.limit = blame_cache_bytes;
            c.shrink(0);
        }
    }

    /// Sets how many changed files the Uncommitted view lists and the largest file it diffs,
    /// in bytes (`limits.worktree_files`, `limits.worktree_diff_kb`).
    pub fn set_worktree_limits(&self, files: usize, diff_bytes: u64) {
        self.worktree_files.store(files, Ordering::Relaxed);
        self.worktree_diff_bytes
            .store(diff_bytes, Ordering::Relaxed);
    }

    pub(crate) fn worktree_files(&self) -> usize {
        self.worktree_files.load(Ordering::Relaxed)
    }

    pub(crate) fn worktree_diff_bytes(&self) -> u64 {
        self.worktree_diff_bytes.load(Ordering::Relaxed)
    }

    /// Sets the threads [`Repo::numstat_many`] spreads a batch over; 0 means one per core.
    #[must_use]
    pub const fn with_workers(mut self, workers: usize) -> Self {
        self.workers = workers;
        self
    }

    /// The text cap in force, see [`Repo::with_limits`].
    pub fn text_limit(&self) -> u64 {
        self.text_limit.load(Ordering::Relaxed)
    }

    /// Path of the `.git` directory, where gitterrace keeps its cache.
    pub fn git_dir(&self) -> &Path {
        self.inner.git_dir()
    }

    /// The worktree folder, canonical; for a bare repository its git dir (or the folder
    /// holding a `.git` dir).
    pub fn root(&self) -> PathBuf {
        let root = self
            .inner
            .work_dir()
            .unwrap_or_else(|| self.inner.git_dir());
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if root.file_name().is_some_and(|n| n == ".git") {
            root.parent().map(Path::to_path_buf).unwrap_or(root)
        } else {
            root
        }
    }

    /// Display name: the worktree folder name.
    pub fn name(&self) -> String {
        self.root().file_name().map_or_else(
            || "repository".to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// Resolves a revision (branch, tag, sha, `HEAD`) to a full commit id.
    pub fn resolve(&self, rev: &str) -> Result<String> {
        Ok(self.resolve_id(&self.repo(), rev)?.to_string())
    }

    /// Branches, tags and remote branches, newest first.
    pub fn refs(&self) -> Result<Vec<RefInfo>> {
        let repo = self.repo();
        let platform = repo.references().map_err(git)?;
        let mut out = Vec::new();
        for reference in platform.all().map_err(git)? {
            let mut reference = reference.map_err(git)?;
            let name = reference.name().as_bstr().to_str_lossy().into_owned();
            let (kind, short) = if let Some(s) = name.strip_prefix("refs/heads/") {
                (RefKind::Branch, s)
            } else if let Some(s) = name.strip_prefix("refs/tags/") {
                (RefKind::Tag, s)
            } else if let Some(s) = name.strip_prefix("refs/remotes/") {
                (RefKind::Remote, s)
            } else {
                continue;
            };
            if short.ends_with("/HEAD") {
                continue;
            }
            let annotation = (kind == RefKind::Tag)
                .then(|| reference.target().try_id().map(ToOwned::to_owned))
                .flatten()
                .and_then(|id| crate::gitdir::annotation(&repo, id));
            let Ok(commit) = reference.peel_to_commit() else {
                continue;
            };
            let time = commit.time().map_or(0, |t| t.seconds);
            out.push(RefInfo {
                name: short.to_string(),
                kind,
                target: commit.id.to_string(),
                time,
                annotation,
            });
        }
        out.sort_by_key(|r| Reverse(r.time));
        Ok(out)
    }

    /// Name of the branch HEAD points at, or `None` when detached.
    pub fn head_branch(&self) -> Option<String> {
        let name = self.repo().head_name().ok()??;
        Some(name.shorten().to_str_lossy().into_owned())
    }

    pub(crate) fn repo(&self) -> gix::Repository {
        let mut repo = self.inner.to_thread_local();
        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
        repo
    }

    /// A full 40-digit commit id parsed directly. Going through `rev_parse` costs a rescan of
    /// the object database per commit, which dominated the index build on large repositories.
    pub(crate) fn full_id(&self, repo: &gix::Repository, id: &str) -> Result<gix::ObjectId> {
        gix::ObjectId::from_hex(id.as_bytes()).or_else(|_| self.resolve_id(repo, id))
    }

    pub(crate) fn resolve_id(&self, repo: &gix::Repository, rev: &str) -> Result<gix::ObjectId> {
        let not_found = || Error::RevNotFound(rev.to_string());
        if REJECTED_REV.iter().any(|p| rev.contains(p)) {
            return Err(not_found());
        }
        let id = if self.committed_only.load(Ordering::Relaxed) {
            committed_rev(repo, rev).ok_or_else(not_found)?
        } else {
            repo.rev_parse_single(rev)
                .map_err(|_| not_found())?
                .detach()
        };
        let id = repo.find_object(id).map_err(|_| not_found())?;
        let commit = id.peel_to_commit().map_err(|_| not_found())?;
        Ok(commit.id)
    }
}

/// [`Repo::committed_only`]'s resolver: a base (`HEAD`, a full id, a branch, tag or remote ref,
/// short or full name) then `~N` / `^N` steps. Ref names cannot hold `~` or `^`, so the first
/// one ends the base. `None` for anything else.
fn committed_rev(repo: &gix::Repository, rev: &str) -> Option<gix::ObjectId> {
    let cut = rev.find(['~', '^']).unwrap_or(rev.len());
    let (base, mut steps) = rev.split_at(cut);
    let mut id = if base == "HEAD" {
        repo.head_id().ok()?.detach()
    } else if base.len() == 40 {
        gix::ObjectId::from_hex(base.as_bytes()).ok()?
    } else {
        let mut reference = ["refs/heads/", "refs/tags/", "refs/remotes/"]
            .iter()
            .find_map(|p| {
                let full = if base.starts_with(p) {
                    base.to_owned()
                } else {
                    format!("{p}{base}")
                };
                repo.try_find_reference(full.as_str()).ok().flatten()
            })?;
        reference.peel_to_id().ok()?.detach()
    };
    while let Some(op) = steps.chars().next() {
        steps = &steps[1..];
        let digits = steps.bytes().take_while(u8::is_ascii_digit).count();
        let n: usize = if digits == 0 {
            1
        } else {
            steps[..digits].parse().ok()?
        };
        steps = &steps[digits..];
        let commit = repo.find_object(id).ok()?.peel_to_commit().ok()?;
        id = match op {
            '~' => {
                let mut c = commit;
                for _ in 0..n {
                    let p = c.parent_ids().next()?.detach();
                    c = repo.find_commit(p).ok()?;
                }
                c.id
            }
            _ if n == 0 => commit.id,
            _ => commit.parent_ids().nth(n - 1)?.detach(),
        };
    }
    Some(id)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_treediff;
