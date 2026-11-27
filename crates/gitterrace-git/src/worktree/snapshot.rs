//! Checkpoints of uncommitted work, the reading half: what the working tree holds now, and a
//! diff against what a checkpoint held. Nothing here writes: the caller keeps the copies.
//!
//! The snapshot lists the files that differ from HEAD (the same set as the Uncommitted view, so
//! ignored files are never in it). A file is copied only when it is text and small: a binary or
//! a large file keeps its content hash and no copy, so a later diff can still say it changed.

use std::collections::{BTreeMap, BTreeSet};

use crate::worktree::{Read, classify, head_blob, is_plain, open, read_worktree, scoped_listing};
use crate::{FileDiff, Repo, Result, WorktreeScope};

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
    /// The paths counted in `unchanged`, sorted: one meaning for the page and the API. A file
    /// saved in the checkpoint and committed exactly as saved is listed too.
    pub unchanged_paths: Vec<String>,
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
    let (paths, files_omitted) = scoped_listing(&r, WorktreeScope::All, repo.worktree_files())?;
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

/// The paths that differ between two trees, both sides of a rename, at most `limit`. The
/// traversal itself stops at the limit and tracks no renames, which would buffer every change
/// first: a rename is its removal and its addition, both listed.
fn tree_paths(
    r: &gix::Repository,
    old: &gix::Tree<'_>,
    new: &gix::Tree<'_>,
    limit: usize,
) -> TreePaths {
    match crate::diff::tree_paths_capped(r, old, new, limit) {
        Ok(w) => TreePaths { paths: w.paths, cut: w.cut, failed: false },
        Err(_) => TreePaths { paths: Vec::new(), cut: false, failed: true },
    }
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
    let (mut paths, now_omitted) = scoped_listing(&r, WorktreeScope::All, repo.worktree_files())?;
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
    let mut all: BTreeSet<&str> =
        paths.iter().map(String::as_str).chain(known.keys().copied()).collect();
    // The union of three independently capped lists can be three times the bound.
    let limit = repo.worktree_files();
    if all.len() > limit {
        all = all.into_iter().take(limit).collect();
        out.files_omitted = true;
    }
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
                        out.unchanged_paths.push(path.to_owned());
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
            None => {
                out.unchanged += 1;
                out.unchanged_paths.push(path.to_owned());
            }
        }
    }
    Ok(out)
}

/// A checkpoint as [`worktree_diff_between`] reads it.
#[derive(Clone, Copy, Debug)]
pub struct Held<'a> {
    pub files: &'a [SnapFile],
    /// The commit the checkpoint was taken at.
    pub head: &'a str,
    /// The checkpoint's own file list may be incomplete.
    pub omitted: bool,
}

/// What a checkpoint held of `path`: its copy, or the file at its commit when it did not list
/// it. The flag is true for a listed file whose copy is gone, which reads as "too big to diff".
fn held_read(
    r: &gix::Repository,
    known: &BTreeMap<&str, &SnapFile>,
    tree: &gix::Tree<'_>,
    path: &str,
    big: u64,
) -> (Read, bool) {
    known.get(path).map_or_else(
        || (head_blob(r, tree, path, big), false),
        |f| match (&f.text, f.hash.as_str()) {
            (Some(t), _) => (Read::data(t.clone()), false),
            (None, "") => (Read::Missing, false),
            (None, _) => (Read::Big, true),
        },
    )
}

/// The content id a side stands for, `None` when it cannot be told (too large to read).
fn side_id(read: &Read, known: &BTreeMap<&str, &SnapFile>, path: &str) -> Option<String> {
    match known.get(path) {
        Some(f) if f.text.is_none() && !f.hash.is_empty() => {
            (f.hash != BIG_HASH).then(|| f.hash.clone())
        }
        _ => match read {
            Read::Data(d) => Some(content_id(d)),
            Read::Missing => Some(String::new()),
            Read::Big | Read::Skip => None,
        },
    }
}

/// Checkpoint `from` against checkpoint `to`: only files whose content differs are listed.
///
/// A file a checkpoint did not list read as its own commit then, so the files the commits
/// between the two heads changed are walked too. A file whose copy is gone on either side is
/// named in `gone` and listed without hunks when its content cannot be shown to match.
/// `baseline_missing` says a checkpoint's commit is not in this repository: its unlisted files
/// were read at the current HEAD instead.
pub fn worktree_diff_between(repo: &Repo, from: &Held<'_>, to: &Held<'_>) -> Result<SinceDiff> {
    let r = open(repo)?;
    let big = repo.worktree_diff_bytes();
    let head = r.head_commit().ok().and_then(|c| c.tree().ok());
    let now_tree = || head.clone().unwrap_or_else(|| r.empty_tree());
    let (a_tree, b_tree) = (baseline_tree(&r, from.head), baseline_tree(&r, to.head));
    let mut out = SinceDiff {
        files_omitted: from.omitted || to.omitted,
        baseline_missing: a_tree.is_none() || b_tree.is_none(),
        ..SinceDiff::default()
    };
    let (a_tree, b_tree) = (a_tree.unwrap_or_else(now_tree), b_tree.unwrap_or_else(now_tree));
    let mut all: BTreeSet<String> =
        from.files.iter().chain(to.files).map(|f| f.path.clone()).collect();
    if from.head != to.head {
        let walked = tree_paths(&r, &a_tree, &b_tree, repo.worktree_files());
        out.files_omitted |= walked.cut;
        out.history_unreadable = walked.failed;
        all.extend(walked.paths);
    }
    if all.len() > repo.worktree_files() {
        all = all.into_iter().take(repo.worktree_files()).collect();
        out.files_omitted = true;
    }
    let ka: BTreeMap<&str, &SnapFile> = from.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let kb: BTreeMap<&str, &SnapFile> = to.files.iter().map(|f| (f.path.as_str(), f)).collect();
    for path in all {
        if !is_plain(&path) {
            continue;
        }
        let (old, old_lost) = held_read(&r, &ka, &a_tree, &path, big);
        let (new, new_lost) = held_read(&r, &kb, &b_tree, &path, big);
        let lost = old_lost || new_lost;
        let same = lost
            && side_id(&old, &ka, &path)
                .is_some_and(|a| side_id(&new, &kb, &path).is_some_and(|b| a == b));
        if same {
            out.unchanged += 1;
            out.unchanged_paths.push(path);
            continue;
        }
        if lost {
            out.gone.push(path.clone());
        }
        match classify(&path, &old, &new) {
            Some(d) => out.files.push(d),
            None if lost => {}
            None => {
                out.unchanged += 1;
                out.unchanged_paths.push(path);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
