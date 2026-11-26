//! Checkpoints of uncommitted work, the reading half: what the working tree holds now, and a
//! diff against what a checkpoint held. Nothing here writes: the caller keeps the copies.
//!
//! The snapshot lists the files that differ from HEAD (the same set as the Uncommitted view, so
//! ignored files are never in it). A file is copied only when it is text and small: a binary or
//! a large file keeps its content hash and no copy, so a later diff can still say it changed.

use std::collections::{BTreeMap, BTreeSet};

use crate::worktree::{
    Read, Scope, classify, head_blob, is_plain, open, read_worktree, scoped_listing,
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

/// The working tree as a checkpoint takes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub files: Vec<SnapFile>,
    /// The scan stopped at `limits.worktree_files`: more changed files may exist than the
    /// snapshot names.
    pub files_omitted: bool,
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
    /// The commit the checkpoint was taken at is not in this repository (a reset, a gc, a
    /// different clone), so the files it did not list were compared with the current HEAD
    /// instead. The comparison may then miss changes the commits in between made.
    pub baseline_missing: bool,
    /// The file list may be incomplete: the scan of the working tree stopped at
    /// `limits.worktree_files`, or the checkpoint's did, or the walk of the commits since the
    /// checkpoint did.
    pub files_omitted: bool,
    /// The commits between the checkpoint and HEAD could not be compared (a missing or
    /// unreadable tree object), so files only they changed may be missing.
    pub history_unreadable: bool,
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

/// Every changed file as it is now.
///
/// `copy_bytes` is the largest file whose bytes are kept and `budget_bytes` the most bytes kept
/// in all: past it a file keeps its hash and no copy, so a worktree of many large text files
/// cannot fill memory before the caller packs and caps them.
pub fn worktree_snapshot(repo: &Repo, copy_bytes: u64, budget_bytes: u64) -> Result<Snapshot> {
    let r = open(repo)?;
    let big = repo.worktree_diff_bytes();
    let Some(root) = r.workdir().and_then(|w| w.canonicalize().ok()) else {
        return Ok(Snapshot::default());
    };
    let mut out = Vec::new();
    let mut kept = 0u64;
    let (paths, files_omitted) = scoped_listing(&r, Scope::All, repo.worktree_files())?;
    for path in paths {
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
    Ok(Snapshot { files: out, files_omitted })
}

/// The tree of commit `hex`: the empty tree for an empty id (the checkpoint was taken before
/// the first commit), `None` when the commit is not here.
fn baseline_tree<'r>(r: &'r gix::Repository, hex: &str) -> Option<gix::Tree<'r>> {
    if hex.is_empty() {
        return Some(r.empty_tree());
    }
    let id = gix::ObjectId::from_hex(hex.as_bytes()).ok()?;
    r.find_commit(id).ok()?.tree().ok()
}

/// What the commits between two trees changed.
struct TreePaths {
    paths: Vec<String>,
    /// The walk stopped at the limit, so more paths may have changed.
    cut: bool,
    /// The trees could not be compared: a missing or unreadable object.
    failed: bool,
}

/// The paths that differ between two trees, both sides of a rename, at most `limit` changes.
fn tree_paths(
    r: &gix::Repository,
    old: &gix::Tree<'_>,
    new: &gix::Tree<'_>,
    limit: usize,
) -> TreePaths {
    use gix::bstr::ByteSlice;
    use gix::object::tree::diff::ChangeDetached as C;

    let failed = TreePaths { paths: Vec::new(), cut: false, failed: true };
    let Ok(mut cache) = crate::diff::resource_cache(r) else {
        return failed;
    };
    let exact = gix::diff::Rewrites { percentage: None, limit: 0, ..Default::default() };
    let Ok((changes, cut)) =
        crate::diff::tree_changes_capped(r, Some(old), new, &mut cache, exact, limit)
    else {
        return failed;
    };
    let mut paths = Vec::new();
    for c in &changes {
        if let C::Rewrite { source_location, .. } = c {
            paths.push(source_location.to_str_lossy().into_owned());
        }
        paths.push(c.location().to_str_lossy().into_owned());
    }
    TreePaths { paths, cut, failed: false }
}

/// The working tree against a checkpoint taken at `base_head`: only files whose content differs
/// are listed.
///
/// A file the checkpoint did not list was equal to `base_head` then, so it is diffed against
/// that commit, and the files the commits since changed are walked too: a file committed after
/// the checkpoint still differs from what it was. A file whose copy is gone is diffed against
/// the current HEAD and named in `gone`. `before_omitted` is the checkpoint's own file-list
/// completeness, carried into `files_omitted`.
pub fn worktree_diff_since(
    repo: &Repo,
    before: &[SnapFile],
    base_head: &str,
    before_omitted: bool,
) -> Result<SinceDiff> {
    let r = open(repo)?;
    let big = repo.worktree_diff_bytes();
    let (mut paths, now_omitted) = scoped_listing(&r, Scope::All, repo.worktree_files())?;
    let head = r.head_commit().ok().and_then(|c| c.tree().ok());
    let mut out =
        SinceDiff { files_omitted: now_omitted || before_omitted, ..SinceDiff::default() };
    let base = baseline_tree(&r, base_head);
    out.baseline_missing = base.is_none();
    if let Some(base) = &base {
        let now = head.clone().unwrap_or_else(|| r.empty_tree());
        let walked = tree_paths(&r, base, &now, repo.worktree_files());
        out.files_omitted |= walked.cut;
        out.history_unreadable = walked.failed;
        paths.extend(walked.paths);
    }
    let root = r.workdir().and_then(|w| w.canonicalize().ok());
    let known: BTreeMap<&str, &SnapFile> = before.iter().map(|f| (f.path.as_str(), f)).collect();
    let all: BTreeSet<&str> =
        paths.iter().map(String::as_str).chain(known.keys().copied()).collect();
    let from_head =
        |path: &str| head.as_ref().map_or(Read::Missing, |t| head_blob(&r, t, path, big));
    let from_base =
        |path: &str| base.as_ref().map_or_else(|| from_head(path), |t| head_blob(&r, t, path, big));
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
            None => (from_base(path), false),
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
