//! Bulk reads at a revision: file lists, blob scanning, commit scanning and per-entry last
//! commits. Each call opens its own repository handle so callers can run them on many threads.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gix::bstr::ByteSlice;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::error::git;
use crate::ident;
use crate::{CommitSummary, Repo, Result};

const OBJECT_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Opaque handle to a blob in the object database.
#[derive(Clone, Copy, Debug)]
pub struct BlobId(gix::ObjectId);

impl BlobId {
    /// Lowercase hex object id, a stable cache key.
    pub fn hex(&self) -> String {
        self.0.to_string()
    }
}

/// A file in a commit's tree.
#[derive(Debug, Clone)]
pub struct FileRef {
    pub path: String,
    pub blob: BlobId,
}

/// The newest commit that changed one tree entry.
#[derive(Debug, Clone)]
pub struct LastCommit {
    pub name: String,
    pub id: String,
    pub subject: String,
    pub time: i64,
}

fn open(repo: &Repo) -> Result<gix::Repository> {
    let mut r = gix::open(repo.git_dir()).map_err(git)?;
    r.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
    Ok(r)
}

fn oid(id: &str) -> Result<gix::ObjectId> {
    gix::ObjectId::from_hex(id.as_bytes()).map_err(git)
}

/// All regular files (not links or submodules) in the tree of `rev`, in tree order.
pub fn files(repo: &Repo, rev: &str) -> Result<Vec<FileRef>> {
    let id = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let tree = r.find_commit(id).map_err(git)?.tree().map_err(git)?;
    let mut rec = gix::traverse::tree::Recorder::default();
    tree.traverse().breadthfirst(&mut rec).map_err(git)?;
    let mut out: Vec<FileRef> = rec
        .records
        .into_iter()
        .filter(|e| e.mode.is_blob())
        .map(|e| FileRef {
            path: e.filepath.to_str_lossy().into_owned(),
            blob: BlobId(e.oid),
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Reads blobs on one thread. Make one per worker.
pub struct BlobReader {
    repo: gix::Repository,
}

impl BlobReader {
    pub fn open(repo: &Repo) -> Result<Self> {
        Ok(Self { repo: open(repo)? })
    }

    /// Content of a text blob, or `None` when it is larger than `max` bytes or binary.
    pub fn text(&self, id: BlobId, max: u64) -> Option<String> {
        if self.repo.find_header(id.0).ok()?.size() > max {
            return None;
        }
        let data = self.repo.find_object(id.0).ok()?.detach().data;
        if crate::diff::is_binary(&data) {
            return None;
        }
        String::from_utf8(data).ok()
    }
}

impl BlobReader {
    /// Number of lines (a last line without newline counts), or `None` for binary content or
    /// blobs larger than `max` bytes.
    pub fn line_count(&self, id: BlobId, max: u64) -> Option<u32> {
        if self.repo.find_header(id.0).ok()?.size() > max {
            return None;
        }
        let data = self.repo.find_object(id.0).ok()?.detach().data;
        if crate::diff::is_binary(&data) {
            return None;
        }
        Some(crate::diff::line_count(&data))
    }
}

/// Object id of the tree at `path` in `rev`, a cheap cache key for directory listings.
pub fn tree_id(repo: &Repo, rev: &str, path: &str) -> Result<String> {
    let id = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let commit = r.find_commit(id).map_err(git)?;
    Ok(dir_oid(&commit, path.trim_matches('/'))?
        .map(|o| o.to_string())
        .unwrap_or_default())
}

/// Visits commits reachable from `rev`, newest first, with the full message. `f` returns
/// `false` to stop.
pub fn scan_commits(
    repo: &Repo,
    rev: &str,
    mut f: impl FnMut(&CommitSummary, &str) -> bool,
) -> Result<()> {
    let tip = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let mailmap = ident::mailmap(&r, tip);
    let walk = r
        .rev_walk([tip])
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
        .all()
        .map_err(git)?;
    for info in walk {
        let commit = info.map_err(git)?.object().map_err(git)?;
        let message = commit
            .message_raw()
            .map_err(git)?
            .to_str_lossy()
            .into_owned();
        let summary = CommitSummary {
            id: commit.id.to_string(),
            parents: commit.parent_ids().map(|p| p.to_string()).collect(),
            author: ident::signature(&mailmap, commit.author().map_err(git)?),
            committer: ident::signature(&mailmap, commit.committer().map_err(git)?),
            subject: message.lines().next().unwrap_or_default().trim().to_owned(),
        };
        if !f(&summary, &message) {
            break;
        }
    }
    Ok(())
}

fn dir_oid(commit: &gix::Commit<'_>, dir: &str) -> Result<Option<gix::ObjectId>> {
    let tree = commit.tree().map_err(git)?;
    if dir.is_empty() {
        return Ok(Some(tree.id));
    }
    Ok(crate::treepath::lookup(&tree, dir)?
        .filter(|e| e.mode().is_tree())
        .map(|e| e.object_id()))
}

fn entries(repo: &gix::Repository, id: Option<gix::ObjectId>) -> HashMap<Vec<u8>, gix::ObjectId> {
    let mut out = HashMap::new();
    if let Some(tree) = id.and_then(|i| repo.find_tree(i).ok()) {
        for e in tree.iter().flatten() {
            out.insert(e.filename().to_vec(), e.oid().to_owned());
        }
    }
    out
}

/// For each entry of the directory `dir` at `rev`, the newest commit that changed it, and whether
/// the answer is final.
///
/// One walk from `rev`, stopping as soon as every entry is resolved, history runs out, or
/// `budget` runs out. Only the last case returns `false`: the missing entries might still be
/// found by a longer walk, so callers must not cache that answer. An entry whose object no commit
/// introduced (a rewritten history) stays missing from a final answer.
pub fn last_commits(
    repo: &Repo,
    rev: &str,
    dir: &str,
    budget: Duration,
) -> Result<(Vec<LastCommit>, bool)> {
    let start = Instant::now();
    let dir = dir.trim_matches('/');
    let tip = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let tip_commit = r.find_commit(tip).map_err(git)?;
    let mut pending = entries(&r, dir_oid(&tip_commit, dir)?);
    let walk = r
        .rev_walk([tip])
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
        .all()
        .map_err(git)?;
    let mut found: Vec<(Vec<u8>, gix::ObjectId)> = Vec::new();
    let mut complete = true;
    for info in walk {
        if pending.is_empty() {
            break;
        }
        if start.elapsed() >= budget {
            complete = false;
            break;
        }
        let commit = info.map_err(git)?.object().map_err(git)?;
        let here = dir_oid(&commit, dir)?;
        let parents: Vec<_> = commit.parent_ids().collect();
        // A root introduces exactly what its tree holds. Other roots (merged unrelated
        // histories) may still own the rest, so the walk goes on.
        if parents.is_empty() {
            let cur = entries(&r, here);
            pending.retain(|n, o| {
                let mine = cur.get(n) == Some(o);
                if mine {
                    found.push((n.clone(), commit.id));
                }
                !mine
            });
            continue;
        }
        let mut parent_dirs = Vec::with_capacity(parents.len());
        for p in parents {
            let pc = r.find_commit(p).map_err(git)?;
            parent_dirs.push(dir_oid(&pc, dir)?);
        }
        // Same directory object as a parent: nothing in it changed here.
        if parent_dirs.contains(&here) {
            continue;
        }
        let maps: Vec<_> = parent_dirs.iter().map(|d| entries(&r, *d)).collect();
        let cur = entries(&r, here);
        let done: Vec<Vec<u8>> = pending
            .iter()
            .filter(|(n, o)| cur.get(*n) == Some(o) && maps.iter().all(|m| m.get(*n) != Some(o)))
            .map(|(n, _)| n.clone())
            .collect();
        for n in done {
            pending.remove(&n);
            found.push((n, commit.id));
        }
    }
    let mut cache: HashMap<gix::ObjectId, (String, i64)> = HashMap::new();
    let mut out = Vec::with_capacity(found.len());
    for (name, id) in found {
        let (subject, time) = if let Some(v) = cache.get(&id) {
            v.clone()
        } else {
            let c = r.find_commit(id).map_err(git)?;
            let subject = c
                .message_raw()
                .map_err(git)?
                .lines()
                .next()
                .unwrap_or_default()
                .to_str_lossy()
                .trim()
                .to_owned();
            let time = c.author().map_err(git)?.time().unwrap_or_default().seconds;
            cache.insert(id, (subject.clone(), time));
            (subject, time)
        };
        out.push(LastCommit {
            name: name.to_str_lossy().into_owned(),
            id: id.to_string(),
            subject,
            time,
        });
    }
    Ok((out, complete))
}
