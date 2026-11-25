//! Checkpoints of uncommitted work, the reading half: what the working tree holds now, and a
//! diff against what a checkpoint held. Nothing here writes: the caller keeps the copies.
//!
//! The snapshot lists the files that differ from HEAD (the same set as the Uncommitted view, so
//! ignored files are never in it). A file is copied only when it is text and small: a binary or
//! a large file keeps its content hash and no copy, so a later diff can still say it changed.

use std::collections::{BTreeMap, BTreeSet};

use crate::worktree::{
    Read, Scope, classify, head_blob, is_plain, open, read_worktree, scoped_paths,
};
use crate::{FileDiff, Repo, Result};

/// The hash of a file too large to read: it matches nothing, so such a file always reads as
/// changed.
pub const BIG_HASH: &str = "big";

/// One changed file of a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapFile {
    pub path: String,
    /// The git blob id of the content, empty for a file that was deleted, [`BIG_HASH`] for one
    /// too large to read.
    pub hash: String,
    /// The bytes, only for a small text file.
    pub text: Option<Vec<u8>>,
}

/// A checkpoint compared with the working tree now.
#[derive(Debug, Default)]
pub struct SinceDiff {
    /// The files whose content differs from the checkpoint, sorted by path.
    pub files: Vec<FileDiff>,
    /// Files that were in the checkpoint or are changed now and read the same.
    pub unchanged: usize,
    /// Files whose checkpoint copy is gone (binary, too large, dropped): diffed against HEAD.
    pub gone: Vec<String>,
}

/// The git blob id of `bytes` as hex. Hashed as SHA-1 whatever the repository uses, so an id
/// stays the same across repositories.
#[must_use]
pub fn content_id(bytes: &[u8]) -> String {
    gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::objs::Kind::Blob, bytes).map_or_else(
        |_| format!("fnv{:016x}", crate::fnv1a64(bytes.iter().copied())),
        |id| id.to_hex().to_string(),
    )
}

/// Every changed file as it is now. `copy_bytes` is the largest file whose bytes are kept and
/// `budget_bytes` the most bytes kept in all: past it a file keeps its hash and no copy, so a
/// worktree of many large text files cannot fill memory before the caller packs and caps them.
pub fn worktree_snapshot(repo: &Repo, copy_bytes: u64, budget_bytes: u64) -> Result<Vec<SnapFile>> {
    let r = open(repo)?;
    let big = repo.worktree_diff_bytes();
    let Some(root) = r.workdir().and_then(|w| w.canonicalize().ok()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut kept = 0u64;
    for path in scoped_paths(&r, Scope::All, repo.worktree_files())? {
        if !is_plain(&path) {
            continue;
        }
        let read = read_worktree(&root, &path, big);
        out.push(match &read {
            Read::Data(d) => {
                let keep = d.len() as u64 <= copy_bytes
                    && kept.saturating_add(d.len() as u64) <= budget_bytes
                    && !crate::diff::is_binary(d);
                if keep {
                    kept += d.len() as u64;
                }
                SnapFile { hash: content_id(d), text: keep.then(|| d.clone()), path }
            }
            Read::Big => SnapFile { path, hash: BIG_HASH.to_owned(), text: None },
            Read::Missing => SnapFile { path, hash: String::new(), text: None },
            Read::Skip => continue,
        });
    }
    Ok(out)
}

/// The working tree against a checkpoint: only files whose content differs are listed.
///
/// A file the checkpoint did not list was equal to HEAD then, so it is diffed against HEAD. A
/// file whose copy is gone is diffed against HEAD too and named in `gone`.
pub fn worktree_diff_since(repo: &Repo, before: &[SnapFile]) -> Result<SinceDiff> {
    let r = open(repo)?;
    let big = repo.worktree_diff_bytes();
    let paths = scoped_paths(&r, Scope::All, repo.worktree_files())?;
    let head = r.head_commit().ok().and_then(|c| c.tree().ok());
    let root = r.workdir().and_then(|w| w.canonicalize().ok());
    let known: BTreeMap<&str, &SnapFile> = before.iter().map(|f| (f.path.as_str(), f)).collect();
    let all: BTreeSet<&str> =
        paths.iter().map(String::as_str).chain(known.keys().copied()).collect();
    let from_head =
        |path: &str| head.as_ref().map_or(Read::Missing, |t| head_blob(&r, t, path, big));
    let mut out = SinceDiff::default();
    for path in all {
        if !is_plain(path) {
            continue;
        }
        let new = root.as_deref().map_or(Read::Skip, |w| read_worktree(w, path, big));
        let (old, gone) = match known.get(path) {
            Some(b) => match (&b.text, b.hash.as_str()) {
                (Some(t), _) => (Read::data(t.clone()), false),
                (None, "") => (Read::Missing, false),
                (None, hash) => {
                    if matches!(&new, Read::Data(d) if content_id(d) == hash) {
                        out.unchanged += 1;
                        continue;
                    }
                    (from_head(path), true)
                }
            },
            None => (from_head(path), false),
        };
        if gone {
            out.gone.push(path.to_owned());
        }
        match classify(path, &old, &new) {
            Some(d) => out.files.push(d),
            None if gone || matches!(new, Read::Skip) => {}
            None => out.unchanged += 1,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
