use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use gix::bstr::ByteSlice;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::error::git;
use crate::ident;
use crate::{
    BlameLine, Blob, CommitDetail, CommitSummary, Error, MAX_TEXT_BYTES, RawBlob, RefInfo, RefKind,
    Result, TreeEntry, TreeEntryKind,
};

/// Decoded-object cache; trees and commits are re-read constantly by walks and diffs.
const OBJECT_CACHE_BYTES: usize = 128 * 1024 * 1024;
/// Object cache of one index worker thread.
const BLAME_OBJECT_CACHE_BYTES: usize = 8 * 1024 * 1024;
const INDEX_WORKER_CACHE_BYTES: usize = 16 * 1024 * 1024;
/// Rev syntax that searches history (`:/text`, `^{/text}`) or reads reflogs (`@{...}`).
/// Every caller passes URL input here, so the refusal lives at the one choke point.
const REJECTED_REV: [&str; 3] = [":/", "^{/", "@{"];

/// A repository opened read-only.
pub struct Repo {
    inner: gix::ThreadSafeRepository,
    blame_cache: Mutex<BlameCache>,
    /// Largest blob read as text by this handle, see [`Repo::with_limits`].
    text_limit: AtomicU64,
    /// Threads [`Repo::numstat_many`] uses; 0 means one per core.
    workers: usize,
    /// Ahead and behind per (branch tip, base): immutable for a pair of commits.
    pub(crate) divergence: Mutex<HashMap<(gix::ObjectId, gix::ObjectId), (usize, usize)>>,
    /// The root-commit id, see `repo_id.rs`.
    pub(crate) id_cache: OnceLock<Option<String>>,
}

/// Blame results kept per (commit id, path), bounded by approximate bytes.
const BLAME_CACHE_BYTES: usize = 32 * 1024 * 1024;

struct BlameCache {
    /// Approximate bytes kept before the oldest entries go.
    limit: usize,
    map: HashMap<(gix::ObjectId, String), Arc<Vec<BlameLine>>>,
    /// Least recently used first.
    order: VecDeque<(gix::ObjectId, String)>,
    bytes: usize,
}

fn blame_size(lines: &[BlameLine]) -> usize {
    lines
        .iter()
        .map(|l| 64 + l.commit.len() + l.author.len() + l.text.len())
        .sum()
}

impl BlameCache {
    fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        }
    }

    fn get(&mut self, key: &(gix::ObjectId, String)) -> Option<Arc<Vec<BlameLine>>> {
        let hit = self.map.get(key)?.clone();
        if let Some(i) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(i)?;
            self.order.push_back(k);
        }
        Some(hit)
    }

    fn put(&mut self, key: (gix::ObjectId, String), value: Arc<Vec<BlameLine>>) {
        let size = blame_size(&value);
        if size > self.limit || self.map.contains_key(&key) {
            return;
        }
        while self.bytes + size > self.limit {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(v) = self.map.remove(&old) {
                self.bytes -= blame_size(&v);
            }
        }
        self.bytes += size;
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
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
            workers: 0,
            divergence: Mutex::new(HashMap::new()),
            id_cache: OnceLock::new(),
        })
    }

    /// Sets the largest blob this handle reads as text (blob views, blame, diffs with hunks)
    /// and the memory the blame cache may keep. The index always uses [`MAX_TEXT_BYTES`], so
    /// its stored numbers do not depend on a display setting.
    #[must_use]
    pub fn with_limits(self, text_bytes: u64, blame_cache_bytes: usize) -> Self {
        self.set_limits(text_bytes, blame_cache_bytes);
        self
    }

    /// [`Repo::with_limits`] on a handle in use. A smaller blame cache shrinks as entries are
    /// added, so memory falls without a stall.
    pub fn set_limits(&self, text_bytes: u64, blame_cache_bytes: usize) {
        self.text_limit.store(text_bytes, Ordering::Relaxed);
        if let Ok(mut c) = self.blame_cache.lock() {
            c.limit = blame_cache_bytes;
        }
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
        Ok(Self::resolve_id(&self.repo(), rev)?.to_string())
    }

    /// Commits reachable from `rev`, newest first, optionally limited to `path`.
    pub fn log(
        &self,
        rev: &str,
        path: Option<&str>,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<CommitSummary>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let path = path.map(|p| p.trim_matches('/')).filter(|p| !p.is_empty());
        let walk = repo
            .rev_walk([tip])
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .all()
            .map_err(git)?;
        let mailmap = ident::mailmap(&repo, tip);
        let mut out = Vec::new();
        let mut seen = 0;
        for info in walk {
            if out.len() >= limit {
                break;
            }
            let info = info.map_err(git)?;
            let commit = info.object().map_err(git)?;
            if let Some(path) = path
                && !touches_path(&repo, &commit, path)?
            {
                continue;
            }
            if seen < skip {
                seen += 1;
                continue;
            }
            out.push(summary(&commit, &mailmap)?);
        }
        Ok(out)
    }

    /// Author times of the commits reachable from `rev` with a commit time at or after `since`,
    /// newest first. Reads only commit headers, so a busy history stays cheap.
    pub fn commit_times_since(&self, rev: &str, since: i64) -> Result<Vec<i64>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let walk = repo
            .rev_walk([tip])
            .sorting(Sorting::ByCommitTimeCutoff {
                order: CommitTimeOrder::NewestFirst,
                seconds: since,
            })
            .all()
            .map_err(git)?;
        let mut out = Vec::new();
        for info in walk {
            let commit = info.map_err(git)?.object().map_err(git)?;
            out.push(commit.author().map_err(git)?.seconds());
        }
        Ok(out)
    }

    /// Commit ids reachable from `rev`, oldest first. Used by the indexer.
    pub fn rev_list(&self, rev: &str) -> Result<Vec<String>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let walk = repo
            .rev_walk([tip])
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .all()
            .map_err(git)?;
        let mut ids = Vec::new();
        for info in walk {
            ids.push(info.map_err(git)?.id.to_string());
        }
        ids.reverse();
        Ok(ids)
    }

    /// One commit with its diff against the first parent.
    pub fn commit(&self, id: &str) -> Result<CommitDetail> {
        let repo = self.repo();
        let oid = Self::resolve_id(&repo, id)?;
        let commit = repo.find_commit(oid).map_err(git)?;
        let mailmap = ident::mailmap(&repo, oid);
        let message = commit
            .message_raw()
            .map_err(git)?
            .to_str_lossy()
            .into_owned();
        let body = message
            .split_once('\n')
            .map(|(_, rest)| rest.trim().to_string())
            .unwrap_or_default();
        Ok(CommitDetail {
            summary: summary(&commit, &mailmap)?,
            body,
            files: crate::diff::changes(&repo, &commit, true, self.text_limit())?,
        })
    }

    /// One file of [`Self::commit`]'s list, by index, without diffing the others.
    pub fn commit_file(&self, id: &str, index: usize) -> Result<Option<crate::FileDiff>> {
        let repo = self.repo();
        let oid = Self::resolve_id(&repo, id)?;
        let commit = repo.find_commit(oid).map_err(git)?;
        crate::diff::change_at(&repo, &commit, index, self.text_limit())
    }

    /// Summaries of exactly these commits, in order. Each commit is mapped through its own
    /// `.mailmap`, parsed once per distinct blob instead of once per commit.
    pub fn summaries(&self, ids: &[String]) -> Result<Vec<CommitSummary>> {
        let repo = self.repo();
        let mut maps: std::collections::HashMap<Option<gix::ObjectId>, gix::mailmap::Snapshot> =
            std::collections::HashMap::new();
        ids.iter()
            .map(|id| {
                let oid = Self::full_id(&repo, id)?;
                let commit = repo.find_commit(oid).map_err(git)?;
                let blob = ident::mailmap_blob(&repo, oid);
                let map = maps
                    .entry(blob)
                    .or_insert_with(|| ident::parse_mailmap(&repo, blob));
                summary(&commit, map)
            })
            .collect()
    }

    /// Per-file line counts against the first parent, without hunk text, for many commits
    /// spread over all cores. Results are in the order of `ids`; the work is CPU bound and
    /// independent, so the indexer batches it.
    /// A commit whose objects cannot be read (a shallow clone's boundary, a pruned or hostile
    /// object store) gets no file stats instead of failing the whole batch.
    pub fn numstat_many(&self, ids: &[String]) -> Result<Vec<Vec<crate::FileDiff>>> {
        let workers = match self.workers {
            0 => std::thread::available_parallelism().map_or(1, usize::from),
            n => n,
        };
        let per = ids.len().div_ceil(workers).max(1);
        std::thread::scope(|scope| {
            let handles: Vec<_> = ids
                .chunks(per)
                .map(|chunk| {
                    scope.spawn(move || {
                        let mut repo = self.repo();
                        // Ten workers each keeping the shared 128 MB cache cost over 1 GB; the
                        // diffs of one commit touch few objects twice.
                        repo.object_cache_size(INDEX_WORKER_CACHE_BYTES);
                        let mut cache = crate::diff::resource_cache(&repo)?;
                        chunk
                            .iter()
                            .map(|id| {
                                let oid = Self::full_id(&repo, id)?;
                                let commit = repo.find_commit(oid).map_err(git)?;
                                Ok(crate::diff::changes_with(
                                    &repo,
                                    &commit,
                                    false,
                                    MAX_TEXT_BYTES,
                                    &mut cache,
                                )
                                .unwrap_or_default())
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                })
                .collect();
            let mut out = Vec::with_capacity(ids.len());
            for handle in handles {
                let part = handle
                    .join()
                    .map_err(|_| Error::Git("numstat worker panicked".to_string()))??;
                out.extend(part);
            }
            Ok(out)
        })
    }

    /// Entries of the directory at `path` in `rev`, dirs first then by name.
    pub fn tree(&self, rev: &str, path: &str) -> Result<Vec<TreeEntry>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let root = repo.find_commit(tip).map_err(git)?.tree().map_err(git)?;
        let path = path.trim_matches('/');
        let tree = if path.is_empty() {
            root
        } else {
            let entry = root
                .lookup_entry_by_path(path)
                .map_err(git)?
                .ok_or_else(|| Error::PathNotFound(path.to_string()))?;
            if !entry.mode().is_tree() {
                return Err(Error::PathNotFound(path.to_string()));
            }
            entry.object().map_err(git)?.into_tree()
        };
        let mut out = Vec::new();
        for entry in tree.iter() {
            let entry = entry.map_err(git)?;
            let mode = entry.mode();
            let kind = if mode.is_tree() {
                TreeEntryKind::Dir
            } else if mode.is_commit() {
                TreeEntryKind::Submodule
            } else if mode.is_link() {
                TreeEntryKind::Symlink
            } else {
                TreeEntryKind::File
            };
            let size = match kind {
                TreeEntryKind::File | TreeEntryKind::Symlink => {
                    repo.find_header(entry.oid()).ok().map(|h| h.size())
                }
                _ => None,
            };
            out.push(TreeEntry {
                name: entry.filename().to_str_lossy().into_owned(),
                kind,
                size,
            });
        }
        out.sort_by(|a, b| {
            let dir = |e: &TreeEntry| e.kind != TreeEntryKind::Dir;
            dir(a).cmp(&dir(b)).then_with(|| a.name.cmp(&b.name))
        });
        Ok(out)
    }

    /// One resolve and one tree lookup for `/raw`: the blob id and size, plus the bytes unless
    /// `skip(id_hex, size)` says they are not needed (too large, or the client has them).
    pub fn raw_blob(
        &self,
        rev: &str,
        path: &str,
        skip: impl FnOnce(&str, u64) -> bool,
    ) -> Result<RawBlob> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let id = Self::blob_id_at(&repo, tip, path.trim_matches('/'))?;
        let size = repo.find_header(id).map_err(git)?.size();
        let id_hex = id.to_string();
        let bytes = if skip(&id_hex, size) {
            None
        } else {
            Some(repo.find_object(id).map_err(git)?.detach().data)
        };
        Ok(RawBlob {
            id: id_hex,
            size,
            bytes,
        })
    }

    /// Raw bytes of the file at `path` in `rev`.
    pub fn blob_bytes(&self, rev: &str, path: &str) -> Result<Vec<u8>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        Self::blob_bytes_at(&repo, tip, path.trim_matches('/'))
    }

    fn blob_bytes_at(repo: &gix::Repository, tip: gix::ObjectId, path: &str) -> Result<Vec<u8>> {
        let id = Self::blob_id_at(repo, tip, path)?;
        Ok(repo.find_object(id).map_err(git)?.detach().data)
    }

    fn blob_id_at(repo: &gix::Repository, tip: gix::ObjectId, path: &str) -> Result<gix::ObjectId> {
        Ok(repo
            .find_commit(tip)
            .map_err(git)?
            .tree()
            .map_err(git)?
            .lookup_entry_by_path(path)
            .map_err(git)?
            .filter(|e| !e.mode().is_tree() && !e.mode().is_commit())
            .ok_or_else(|| Error::PathNotFound(path.to_string()))?
            .object_id())
    }

    /// The file at `path` in `rev`. `text` is `None` for binary blobs and for blobs over
    /// [`MAX_TEXT_BYTES`], which are never read.
    pub fn blob(&self, rev: &str, path: &str) -> Result<Blob> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let path = path.trim_matches('/');
        let id = Self::blob_id_at(&repo, tip, path)?;
        let size = repo.find_header(id).map_err(git)?.size();
        let text = if size > self.text_limit() {
            None
        } else {
            let data = repo.find_object(id).map_err(git)?.detach().data;
            if crate::diff::is_binary(&data) {
                None
            } else {
                String::from_utf8(data).ok()
            }
        };
        Ok(Blob {
            path: path.to_string(),
            size,
            text,
        })
    }

    pub fn blame(&self, rev: &str, path: &str) -> Result<Vec<BlameLine>> {
        let mut repo = self.repo();
        // Blame touches every ancestor's trees once, so a big cache only adds resident memory.
        repo.object_cache_size(BLAME_OBJECT_CACHE_BYTES);
        let tip = Self::resolve_id(&repo, rev)?;
        let path = path.trim_matches('/');
        let key = (tip, path.to_string());
        if let Some(hit) = self.blame_cache.lock().ok().and_then(|mut c| c.get(&key)) {
            return Ok(hit.as_ref().clone());
        }
        let id = Self::blob_id_at(&repo, tip, path)?;
        // Blame of a huge file costs as much as its diff against every ancestor.
        if repo.find_header(id).map_err(git)?.size() > self.text_limit() {
            return Ok(Vec::new());
        }
        let data = repo.find_object(id).map_err(git)?.detach().data;
        if crate::diff::is_binary(&data) {
            return Ok(Vec::new());
        }
        let Ok(text) = String::from_utf8(data) else {
            return Ok(Vec::new());
        };
        let outcome = repo
            .blame_file(
                path.into(),
                tip,
                gix::repository::blame_file::Options::default(),
            )
            .map_err(git)?;
        let mailmap = ident::mailmap(&repo, tip);
        let mut commits: HashMap<gix::ObjectId, (String, i64)> = HashMap::new();
        let mut lines: Vec<BlameLine> = Vec::new();
        let source: Vec<&str> = text.split_inclusive('\n').collect();
        for entry in &outcome.entries {
            let (author, time) = if let Some(known) = commits.get(&entry.commit_id) {
                known.clone()
            } else {
                let commit = repo.find_commit(entry.commit_id).map_err(git)?;
                let sig = ident::signature(&mailmap, commit.author().map_err(git)?);
                let value = (sig.name, sig.time);
                commits.insert(entry.commit_id, value.clone());
                value
            };
            for offset in 0..entry.len.get() {
                let line_no = entry.start_in_blamed_file + offset;
                let raw = source.get(line_no as usize).copied().unwrap_or_default();
                lines.push(BlameLine {
                    line_no: line_no + 1,
                    commit: entry.commit_id.to_string(),
                    author: author.clone(),
                    time,
                    text: raw.trim_end_matches(['\n', '\r']).to_string(),
                });
            }
        }
        lines.sort_by_key(|l| l.line_no);
        if let Ok(mut c) = self.blame_cache.lock() {
            c.put(key, Arc::new(lines.clone()));
        }
        Ok(lines)
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
    pub(crate) fn full_id(repo: &gix::Repository, id: &str) -> Result<gix::ObjectId> {
        gix::ObjectId::from_hex(id.as_bytes()).or_else(|_| Self::resolve_id(repo, id))
    }

    pub(crate) fn resolve_id(repo: &gix::Repository, rev: &str) -> Result<gix::ObjectId> {
        let not_found = || Error::RevNotFound(rev.to_string());
        if REJECTED_REV.iter().any(|p| rev.contains(p)) {
            return Err(not_found());
        }
        let id = repo.rev_parse_single(rev).map_err(|_| not_found())?;
        let commit = id
            .object()
            .map_err(git)?
            .peel_to_commit()
            .map_err(|_| not_found())?;
        Ok(commit.id)
    }
}

fn summary(commit: &gix::Commit<'_>, mailmap: &gix::mailmap::Snapshot) -> Result<CommitSummary> {
    let message = commit.message_raw().map_err(git)?;
    let subject = message
        .lines()
        .next()
        .unwrap_or_default()
        .to_str_lossy()
        .trim()
        .to_string();
    Ok(CommitSummary {
        id: commit.id.to_string(),
        parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        author: ident::signature(mailmap, commit.author().map_err(git)?),
        committer: ident::signature(mailmap, commit.committer().map_err(git)?),
        subject,
    })
}

/// History simplification as `git log -- path`: a commit counts when the path's object differs
/// from every parent (a root counts when the path exists).
fn touches_path(repo: &gix::Repository, commit: &gix::Commit<'_>, path: &str) -> Result<bool> {
    let here = path_id(commit, path)?;
    let mut parents = commit.parent_ids().peekable();
    if parents.peek().is_none() {
        return Ok(here.is_some());
    }
    for parent in parents {
        let parent = repo.find_commit(parent).map_err(git)?;
        if path_id(&parent, path)? == here {
            return Ok(false);
        }
    }
    Ok(true)
}

fn path_id(commit: &gix::Commit<'_>, path: &str) -> Result<Option<gix::ObjectId>> {
    let tree = commit.tree().map_err(git)?;
    let entry = tree.lookup_entry_by_path(path).map_err(git)?;
    Ok(entry.map(|e| e.object_id()))
}

#[cfg(test)]
mod tests {
    /// Pinned author and committer date for every commit in this module.
    const T0: &str = "2024-01-01T00:00:00+00:00";

    use gitterrace_testkit::git_at;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::Arc;

    use super::BlameCache;
    use crate::{ChangeKind, LineKind, Repo};

    fn write(dir: &std::path::Path, path: &str, body: &str) {
        let full = dir.join(path);
        fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        fs::write(full, body).expect("write");
    }

    fn commit(dir: &std::path::Path, msg: &str) {
        git_at(dir, T0, &["add", "-A"]);
        git_at(dir, T0, &["commit", "-q", "-m", msg]);
    }

    fn numbered(n: usize) -> String {
        (1..=n).fold(String::new(), |acc, i| acc + &format!("line {i}\n"))
    }

    fn fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = tmp.path();
        git_at(d, T0, &["init", "-q", "-b", "main"]);
        git_at(d, T0, &["config", "user.name", "Old Name"]);
        git_at(d, T0, &["config", "user.email", "old@example.com"]);
        write(
            d,
            ".mailmap",
            "New Name <new@example.com> <old@example.com>\n",
        );
        write(d, "a.txt", &numbered(40));
        write(d, "src/b.txt", "one\ntwo\n");
        commit(d, "feat: first\n\nbody text\nmore body");
        let mut edited = numbered(40)
            .replace("line 5\n", "five\n")
            .replace("line 30\n", "thirty\n");
        edited.push_str("tail\n");
        write(d, "a.txt", &edited);
        write(d, "bin.dat", "\0\u{1}\u{2}binary");
        commit(d, "fix: edit a, add binary");
        fs::rename(d.join("src/b.txt"), d.join("src/c.txt")).expect("rename");
        commit(d, "refactor: rename b to c");
        fs::remove_file(d.join("a.txt")).expect("rm");
        write(d, "src/c.txt", "one\ntwo\nthree\n");
        commit(d, "chore: drop a, grow c");
        tmp
    }

    /// Parses `git log --numstat -z -M` into id to path to `(add, del, old_path)`.
    #[expect(
        clippy::type_complexity,
        reason = "test reference shape, a named type adds nothing"
    )]
    fn reference(
        dir: &std::path::Path,
    ) -> BTreeMap<String, BTreeMap<String, (u32, u32, Option<String>)>> {
        let raw = git_at(
            dir,
            T0,
            &[
                "log",
                "--numstat",
                "-z",
                "-M",
                "--format=%x01%H%x00",
                "--topo-order",
            ],
        );
        let mut out = BTreeMap::new();
        for chunk in raw.split('\u{1}').filter(|c| !c.is_empty()) {
            let mut parts = chunk.split('\0');
            let id = parts.next().expect("id").to_string();
            let mut files = BTreeMap::new();
            while let Some(head) = parts.next() {
                let head = head.trim_start_matches('\n');
                if head.is_empty() {
                    continue;
                }
                let mut f = head.split('\t');
                let (a, d, p) = (
                    f.next().expect("a"),
                    f.next().expect("d"),
                    f.next().expect("p"),
                );
                let (path, old) = if p.is_empty() {
                    let old = parts.next().expect("old").to_string();
                    (parts.next().expect("new").to_string(), Some(old))
                } else {
                    (p.to_string(), None)
                };
                let num = |s: &str| s.parse().unwrap_or(0);
                files.insert(path, (num(a), num(d), old));
            }
            out.insert(id, files);
        }
        out
    }

    #[test]
    fn numstat_matches_git() {
        let tmp = fixture();
        let repo = Repo::discover(tmp.path()).expect("open");
        let want = reference(tmp.path());
        assert_eq!(want.len(), 4);
        for id in repo.rev_list("HEAD").expect("rev_list") {
            let got: BTreeMap<_, _> = repo
                .numstat_many(std::slice::from_ref(&id))
                .expect("numstat")
                .remove(0)
                .into_iter()
                .map(|f| (f.path, (f.additions, f.deletions, f.old_path)))
                .collect();
            assert_eq!(got, want[&id], "commit {id}");
        }
    }

    #[test]
    fn commit_detail_has_hunks_kinds_and_body() {
        let tmp = fixture();
        let repo = Repo::discover(tmp.path()).expect("open");
        let log = repo.log("HEAD", None, 0, 10).expect("log");
        assert_eq!(log.len(), 4);
        let second = &log[2];
        assert_eq!(second.subject, "fix: edit a, add binary");
        let detail = repo.commit(&second.id).expect("commit");
        let a = detail
            .files
            .iter()
            .find(|f| f.path == "a.txt")
            .expect("a.txt");
        assert_eq!(
            (a.kind, a.additions, a.deletions),
            (ChangeKind::Modified, 3, 2)
        );
        assert_eq!(a.hunks.len(), 3);
        let first = &a.hunks[0];
        assert_eq!(first.header, "@@ -2,7 +2,7 @@");
        let removed = first
            .lines
            .iter()
            .find(|l| l.kind == LineKind::Removed)
            .expect("removed");
        assert_eq!(
            (removed.old_no, removed.new_no, removed.text.as_str()),
            (Some(5), None, "line 5")
        );
        let bin = detail
            .files
            .iter()
            .find(|f| f.path == "bin.dat")
            .expect("bin");
        assert!(bin.binary && bin.hunks.is_empty() && bin.kind == ChangeKind::Added);

        let root = repo.commit(&log[3].id).expect("root");
        assert_eq!(root.body, "body text\nmore body");
        assert!(root.files.iter().all(|f| f.kind == ChangeKind::Added));

        let rename = repo.commit(&log[1].id).expect("rename");
        assert_eq!(rename.files.len(), 1);
        assert_eq!(rename.files[0].kind, ChangeKind::Renamed);
        assert_eq!(rename.files[0].old_path.as_deref(), Some("src/b.txt"));
    }

    #[test]
    fn log_orders_filters_and_pages() {
        let tmp = fixture();
        let repo = Repo::discover(tmp.path()).expect("open");
        let all = repo.log("main", None, 0, 100).expect("log");
        assert_eq!(all[0].subject, "chore: drop a, grow c");
        let paged = repo.log("main", None, 1, 2).expect("log");
        assert_eq!(paged.len(), 2);
        assert_eq!(paged[0].id, all[1].id);
        let on_a = repo.log("HEAD", Some("a.txt"), 0, 100).expect("log");
        let subjects: Vec<_> = on_a.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(
            subjects,
            [
                "chore: drop a, grow c",
                "fix: edit a, add binary",
                "feat: first"
            ]
        );
        let oldest_first = repo.rev_list("HEAD").expect("rev_list");
        assert_eq!(oldest_first.last(), Some(&all[0].id));
        assert!(matches!(
            repo.resolve("nope"),
            Err(crate::Error::RevNotFound(_))
        ));
        // History searches and reflog reads never resolve, whichever route passes them.
        for rev in [":/first", "HEAD^{/first}", "main@{1}", "@{-1}"] {
            assert!(
                matches!(repo.resolve(rev), Err(crate::Error::RevNotFound(_))),
                "{rev}"
            );
            assert!(repo.log(rev, None, 0, 1).is_err(), "{rev}");
        }
        assert!(repo.resolve("HEAD~1").is_ok());
    }

    #[test]
    fn mailmap_is_applied() {
        let tmp = fixture();
        let repo = Repo::discover(tmp.path()).expect("open");
        let log = repo.log("HEAD", None, 0, 1).expect("log");
        assert_eq!(log[0].author.name, "New Name");
        assert_eq!(log[0].author.email, "new@example.com");
    }

    #[test]
    fn tree_blob_blame_refs() {
        let tmp = fixture();
        let d = tmp.path();
        git_at(d, T0, &["tag", "v1"]);
        let repo = Repo::discover(d).expect("open");
        let first = repo.log("HEAD", None, 0, 10).expect("log")[3].id.clone();
        let entries = repo.tree(&first, "").expect("tree");
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", ".mailmap", "a.txt"]);
        assert_eq!(repo.tree("HEAD", "src").expect("src").len(), 1);
        assert!(repo.tree("HEAD", "missing").is_err());
        let blob = repo.blob(&first, "src/b.txt").expect("blob");
        assert_eq!(blob.text.as_deref(), Some("one\ntwo\n"));
        assert!(repo.blob("HEAD", "src/c.txt").is_ok());
        let second = repo.log("HEAD", None, 0, 10).expect("log")[2].id.clone();
        assert!(repo.blob(&second, "bin.dat").expect("bin").text.is_none());

        let blame = repo.blame(&second, "a.txt").expect("blame");
        assert_eq!(blame.len(), 41);
        assert_eq!(blame[4].text, "five");
        assert_ne!(blame[4].commit, blame[0].commit);
        assert_eq!(blame[0].author, "New Name");

        assert_eq!(repo.head_branch().as_deref(), Some("main"));
        let refs = repo.refs().expect("refs");
        let kinds: Vec<_> = refs.iter().map(|r| (r.name.as_str(), r.kind)).collect();
        assert!(kinds.contains(&("main", crate::RefKind::Branch)));
        assert!(kinds.contains(&("v1", crate::RefKind::Tag)));
        assert!(!repo.name().is_empty());
    }

    #[test]
    fn a_lower_text_cap_hides_text_and_a_small_blame_cache_keeps_nothing() {
        let tmp = fixture();
        let d = tmp.path();
        let open = Repo::discover(d).expect("open");
        let at = open.log("HEAD", None, 0, 10).expect("log")[2].id.clone();
        assert!(open.blob(&at, "a.txt").expect("blob").text.is_some());
        let tiny = Repo::discover(d).expect("open").with_limits(10, 1);
        assert_eq!(tiny.text_limit(), 10);
        let blob = tiny.blob(&at, "a.txt").expect("blob");
        assert!(blob.size > 10 && blob.text.is_none(), "over the cap");
        assert!(tiny.blame(&at, "a.txt").expect("blame").is_empty());
        let mut cache = BlameCache::with_limit(1);
        let key = (
            gix::ObjectId::empty_tree(gix::hash::Kind::Sha1),
            "p".to_owned(),
        );
        cache.put(
            key.clone(),
            Arc::new(open.blame(&at, "a.txt").expect("blame")),
        );
        assert!(
            cache.get(&key).is_none(),
            "a result over the budget is not kept"
        );
        let mut roomy = BlameCache::with_limit(1 << 20);
        roomy.put(
            key.clone(),
            Arc::new(open.blame(&at, "a.txt").expect("blame")),
        );
        assert!(roomy.get(&key).is_some());
    }

    #[test]
    fn blame_cache_follows_the_commit() {
        let tmp = fixture();
        let d = tmp.path();
        let repo = Repo::discover(d).expect("open");
        let second = repo.log("HEAD", None, 0, 10).expect("log")[2].id.clone();
        let before = repo.blame(&second, "a.txt").expect("blame");
        assert_eq!(
            repo.blame(&second, "a.txt").expect("cached").len(),
            before.len()
        );
        git_at(d, T0, &["checkout", "-q", "-b", "other", &second]);
        std::fs::write(d.join("a.txt"), "only\n").expect("write");
        git_at(d, T0, &["commit", "-qam", "shrink"]);
        let after = repo.blame("other", "a.txt").expect("blame");
        assert_eq!((before.len(), after.len()), (41, 1));
        assert_eq!(repo.blame(&second, "a.txt").expect("old").len(), 41);
    }

    #[test]
    fn blobs_over_the_text_cap_are_never_read_as_text() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = tmp.path();
        git_at(d, T0, &["init", "-q", "-b", "main"]);
        git_at(d, T0, &["config", "user.name", "A"]);
        git_at(d, T0, &["config", "user.email", "a@x"]);
        let size = crate::MAX_TEXT_BYTES as usize + 2;
        write(d, "big.txt", &"a\n".repeat(size / 2));
        commit(d, "add big");
        write(d, "big.txt", &"b\n".repeat(size / 2));
        commit(d, "rewrite big");
        let repo = Repo::discover(d).expect("open");
        let blob = repo.blob("HEAD", "big.txt").expect("blob");
        assert_eq!((blob.size, blob.text), (size as u64, None));
        assert!(repo.blame("HEAD", "big.txt").expect("blame").is_empty());
        let file = &repo.commit("HEAD").expect("commit").files[0];
        assert!(file.binary && file.hunks.is_empty());
        assert_eq!((file.additions, file.deletions), (0, 0));
    }
}
