//! Line blame and its per-handle cache.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::git;
use crate::ident;
use crate::repo::{BLAME_OBJECT_CACHE_BYTES, Repo};
use crate::{BlameLine, Result};

/// Blame results kept per (commit id, path), bounded by approximate bytes.
pub(super) const BLAME_CACHE_BYTES: usize = 32 * 1024 * 1024;
/// Bytes of past versions one blame may read. gix keeps every version it diffed in its blob
/// cache until the blame ends and offers no hook to clear it per step, so without a bound a
/// 2 MB file with 500 versions holds 1 GB. Lines older than the oldest version that fits are
/// blamed on the boundary commit, as `git blame --since` does.
pub(crate) const BLAME_HISTORY_BYTES: u64 = 96 * 1024 * 1024;
const HORIZON_PACK_CACHE_BYTES: usize = 8 * 1024 * 1024;

/// What [`history_horizon`] found.
#[derive(Debug, Default)]
struct Horizon {
    /// The commit time blame must not look past, `None` when the whole history fits.
    since: Option<i64>,
    /// Commits the walk visited: the cost a cold blame pays before gix starts.
    commits: usize,
}

pub(super) struct BlameCache {
    /// Approximate bytes kept before the oldest entries go.
    pub(super) limit: usize,
    map: HashMap<(gix::ObjectId, String), Arc<Vec<BlameLine>>>,
    order: VecDeque<(gix::ObjectId, String)>,
    bytes: usize,
}

fn blame_size(lines: &[BlameLine]) -> usize {
    lines.iter().map(|l| 64 + l.commit.len() + l.author.len() + l.text.len()).sum()
}

impl BlameCache {
    pub(super) fn with_limit(limit: usize) -> Self {
        Self { limit, map: HashMap::new(), order: VecDeque::new(), bytes: 0 }
    }

    pub(super) fn get(&mut self, key: &(gix::ObjectId, String)) -> Option<Arc<Vec<BlameLine>>> {
        let hit = self.map.get(key)?.clone();
        if let Some(i) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(i)?;
            self.order.push_back(k);
        }
        Some(hit)
    }

    /// Drops the oldest entries until `room` more bytes fit under the limit.
    pub(super) fn shrink(&mut self, room: usize) {
        while self.bytes + room > self.limit {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(v) = self.map.remove(&old) {
                self.bytes -= blame_size(&v);
            }
        }
    }

    pub(super) fn put(&mut self, key: (gix::ObjectId, String), value: Arc<Vec<BlameLine>>) {
        let size = blame_size(&value);
        if size > self.limit || self.map.contains_key(&key) {
            return;
        }
        self.shrink(size);
        self.bytes += size;
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
}

impl Repo {
    pub fn blame(&self, rev: &str, path: &str) -> Result<Vec<BlameLine>> {
        let mut repo = self.repo();
        // Blame touches every ancestor's trees once, so a big cache only adds resident memory.
        repo.object_cache_size(BLAME_OBJECT_CACHE_BYTES);
        let tip = self.resolve_id(&repo, rev)?;
        let path = path.trim_matches('/');
        let key = (tip, path.to_string());
        let id = Self::blob_id_at(&repo, tip, path)?;
        // Blame of a huge file costs as much as its diff against every ancestor. Checked before
        // the cache, because the text limit can change at runtime and is not part of the key.
        if repo.find_header(id).map_err(git)?.size() > self.text_limit() {
            return Ok(Vec::new());
        }
        if let Some(hit) = self.blame_cache.lock().ok().and_then(|mut c| c.get(&key)) {
            return Ok(hit.as_ref().clone());
        }
        let data = repo.find_object(id).map_err(git)?.detach().data;
        if crate::diff::is_binary(&data) {
            return Ok(Vec::new());
        }
        let Ok(text) = String::from_utf8(data) else {
            return Ok(Vec::new());
        };
        let outcome = blame_outcome(&repo, tip, path, BLAME_HISTORY_BYTES)?;
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

    /// True when [`Repo::blame`] of `path` at `rev` would be served from the cache.
    pub fn blame_is_cached(&self, rev: &str, path: &str) -> bool {
        let repo = self.repo();
        let Ok(tip) = self.resolve_id(&repo, rev) else {
            return false;
        };
        let key = (tip, path.trim_matches('/').to_string());
        self.blame_cache.lock().is_ok_and(|c| c.map.contains_key(&key))
    }

    /// Empties the blame cache and returns whether it held anything. A server calls this when
    /// nobody browses, so an idle process keeps no blame.
    pub fn clear_blame_cache(&self) -> bool {
        self.blame_cache.lock().is_ok_and(|mut c| {
            let held = !c.map.is_empty();
            c.map = HashMap::new();
            c.order = VecDeque::new();
            c.bytes = 0;
            held
        })
    }

    /// Bytes the blame cache holds and may hold, so a prewarm can leave room for visitors.
    pub fn blame_cache_fill(&self) -> (usize, usize) {
        self.blame_cache.lock().map_or((0, 0), |c| (c.bytes, c.limit))
    }
}

/// Blame of `path` at `tip` with the options `Repository::blame_file` uses by default, except
/// that history stops where the versions of `path` would exceed `budget` bytes (see
/// [`history_horizon`]). That shortcut builds its diff cache through the repository's shared
/// index snapshot, which keeps the whole HEAD index alive for the life of the server; this one
/// owns its index.
pub(crate) fn blame_outcome(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    path: &str,
    budget: u64,
) -> Result<gix::blame::Outcome> {
    Ok(blame_bounded(repo, tip, path, budget)?.0)
}

/// [`blame_outcome`] and the commits the horizon walk visited before the bounded blame. Most
/// files fit the budget, so the blame first runs with no horizon, counting the versions it
/// reads, and gives up at the first one past the budget. The horizon walk runs beside it on
/// its own thread: when the blame fits, the walk stops and nothing reruns; when it does not,
/// the blame gives up as soon as the walk has found the horizon, so a file past the cap pays
/// the walk and the bounded blame, not a whole aborted pass before the walk starts.
fn blame_bounded(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    path: &str,
    budget: u64,
) -> Result<(gix::blame::Outcome, usize)> {
    let blamed = AtomicBool::new(false);
    let found = AtomicBool::new(false);
    // gix::Repository is not Send: the walk opens its own handle from the shared one.
    let shared = repo.clone().into_sync();
    let (first, over, horizon) = std::thread::scope(|s| {
        let walk = s.spawn(|| {
            let walk_repo = shared.to_thread_local();
            let horizon = history_horizon(&walk_repo, tip, path, budget, &blamed);
            if matches!(&horizon, Ok(h) if h.since.is_some()) {
                found.store(true, Ordering::Relaxed);
            }
            horizon
        });
        let counted = Budgeted::new(&repo.objects, budget, &found);
        let first = run_blame(repo, &counted, tip, path, None);
        let over = counted.over.get();
        if !over {
            blamed.store(true, Ordering::Relaxed);
        }
        let horizon = walk.join().unwrap_or_else(|_| Err(git("blame horizon walk panicked")));
        (first, over, horizon)
    });
    match first {
        Ok(outcome) => return Ok((outcome, 0)),
        Err(e) if !over => return Err(e),
        Err(_) => {}
    }
    let horizon = horizon?;
    let since = horizon.since.map(|seconds| gix::date::Time { seconds, offset: 0 });
    let outcome = run_blame(repo, &repo.objects, tip, path, since)?;
    Ok((outcome, horizon.commits))
}

fn run_blame(
    repo: &gix::Repository,
    odb: impl gix::objs::Find + gix::objs::FindHeader,
    tip: gix::ObjectId,
    path: &str,
    since: Option<gix::date::Time>,
) -> Result<gix::blame::Outcome> {
    let mut cache = crate::diff::resource_cache(repo)?;
    gix::blame::file(
        odb,
        gix::blame::Start::Commit(tip),
        repo.commit_graph_if_enabled().map_err(git)?,
        &mut cache,
        path.into(),
        gix::blame::Options {
            diff_algorithm: repo.diff_algorithm().map_err(git)?,
            ranges: gix::blame::BlameRanges::default(),
            since,
            rewrites: None,
            debug_track_path: false,
        },
    )
    .map_err(git)
}

/// An object store that sums the distinct blobs read through it and fails the read that takes
/// the sum past `budget`, or any blob read once `found` says the horizon walk already proved
/// the history does not fit, so a blame gives up before it holds more than the cap.
struct Budgeted<'a> {
    odb: &'a gix::OdbHandle,
    budget: u64,
    held: Cell<u64>,
    seen: RefCell<HashSet<gix::ObjectId>>,
    over: Cell<bool>,
    found: &'a AtomicBool,
}

#[derive(Debug, thiserror::Error)]
#[error("blame history exceeds its byte budget")]
struct OverBudget;

impl<'a> Budgeted<'a> {
    fn new(odb: &'a gix::OdbHandle, budget: u64, found: &'a AtomicBool) -> Self {
        Self {
            odb,
            budget,
            held: Cell::new(0),
            seen: RefCell::new(HashSet::new()),
            over: Cell::new(false),
            found,
        }
    }
}

impl gix::objs::Find for Budgeted<'_> {
    fn try_find<'b>(
        &self,
        id: &gix::hash::oid,
        buffer: &'b mut Vec<u8>,
    ) -> gix::error::Result<Option<gix::objs::Data<'b>>> {
        let data = gix::objs::Find::try_find(self.odb, id, buffer)?;
        if let Some(d) = &data
            && d.kind == gix::objs::Kind::Blob
            && self.seen.borrow_mut().insert(id.to_owned())
        {
            let held = self.held.get() + d.data.len() as u64;
            self.held.set(held);
            if held > self.budget || self.found.load(Ordering::Relaxed) {
                self.over.set(true);
                return Err(gix::error::Error::from_error(OverBudget));
            }
        }
        Ok(data)
    }
}

impl gix::objs::FindHeader for Budgeted<'_> {
    fn try_header(&self, id: &gix::hash::oid) -> gix::error::Result<Option<gix::objs::Header>> {
        gix::objs::FindHeader::try_header(self.odb, id)
    }
}

/// The commit time blame must not look past so the distinct versions of `path` it reads sum
/// to at most `budget` bytes, or `None` when the whole history fits. Walks ancestors newest
/// first, as blame does, and stops at the first version that does not fit. Renames are not
/// followed, matching `rewrites: None`. Gives up with no horizon once `stop` is set: the
/// blame it runs beside has finished within the budget.
fn history_horizon(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    path: &str,
    budget: u64,
    stop: &AtomicBool,
) -> Result<Horizon> {
    // The walk reads each ancestor's root tree and the trees on `path`. Decoded through the
    // repository's caches they stay resident after the blame (the blame prewarm runs this on
    // many files at startup), so it reads raw bytes into two reused buffers with no object
    // cache, and skips commits whose root tree it already looked up.
    let odb = horizon_odb(repo);
    let graph = repo.commit_graph_if_enabled().map_err(git)?;
    let mut commit_buf = Vec::new();
    let mut tree_buf = Vec::new();
    let mut trees = HashMap::new();
    let mut seen = HashSet::new();
    let mut queued = HashSet::from([tip]);
    // Newest first, as blame walks, and pruned as blame prunes: a commit without `path` ends
    // its line (no renames are followed), and a commit whose parent holds the same version
    // passes everything to that one parent, so the other sides of a merge are never read.
    // Walking every ancestor instead read each commit since the file was added, ninety
    // seconds for a header added once to linux.
    let mut queue = std::collections::BinaryHeap::new();
    let mut parents = Vec::new();
    let (time, root) = commit_node(&odb, graph.as_ref(), tip, &mut commit_buf, &mut parents)?;
    let mut out = Horizon::default();
    let Some(first) = blob_at(&odb, root, path, &mut tree_buf, &mut trees)? else {
        return Ok(out);
    };
    queue.push((time, tip, first));
    let mut held = 0u64;
    while let Some((time, id, blob)) = queue.pop() {
        if stop.load(Ordering::Relaxed) {
            return Ok(Horizon::default());
        }
        out.commits += 1;
        commit_node(&odb, graph.as_ref(), id, &mut commit_buf, &mut parents)?;
        let mut sides = Vec::new();
        for parent in std::mem::take(&mut parents) {
            let (t, root) =
                commit_node(&odb, graph.as_ref(), parent, &mut commit_buf, &mut Vec::new())?;
            if let Some(b) = blob_at(&odb, root, path, &mut tree_buf, &mut trees)? {
                sides.push((t, parent, b));
            }
        }
        if let Some(same) = sides.iter().position(|s| s.2 == blob) {
            let keep = sides.swap_remove(same);
            sides.clear();
            sides.push(keep);
        }
        if seen.insert(blob) {
            held +=
                gix::objs::FindHeader::try_header(&odb, &blob).map_err(git)?.map_or(0, |h| h.size);
            if held > budget {
                // Blame skips commits strictly older than `since`: one past this commit's
                // time keeps every newer version and drops this one.
                out.since = Some(time + 1);
                return Ok(out);
            }
        }
        for side in sides {
            if queued.insert(side.1) {
                queue.push(side);
            }
        }
    }
    Ok(out)
}

/// The committer time and root tree of commit `id`, with its parents in `parents`. Read from
/// the commit-graph when it holds `id`: that costs no object read, where decoding the commit
/// from a pack costs a delta chain.
fn commit_node(
    odb: &gix::OdbHandle,
    graph: Option<&gix::commitgraph::Graph>,
    id: gix::ObjectId,
    buf: &mut Vec<u8>,
    parents: &mut Vec<gix::ObjectId>,
) -> Result<(i64, gix::ObjectId)> {
    parents.clear();
    if let Some(commit) = graph.and_then(|g| g.commit_by_id(id)) {
        let g = graph.ok_or_else(|| git("commit-graph vanished"))?;
        for pos in commit.iter_parents() {
            parents.push(g.id_at(pos.map_err(git)?).to_owned());
        }
        let time = i64::try_from(commit.committer_timestamp()).map_err(git)?;
        return Ok((time, commit.root_tree_id().to_owned()));
    }
    let data = gix::objs::Find::try_find(odb, &id, buf)
        .map_err(git)?
        .ok_or_else(|| git(format!("missing commit {id}")))?;
    let mut iter = gix::objs::CommitRefIter::from_bytes(data.data, data.object_hash);
    let root = iter.tree_id().map_err(git)?;
    parents.extend(iter.parent_ids());
    let time = iter.committer().map_err(git)?.time().map_err(git)?.seconds;
    Ok((time, root))
}

/// The object store the horizon walk reads through: the repository's, with no object cache
/// and a pack delta cache of its own, so nothing it reads outlives the walk. Without a delta
/// cache every tree rebuilds its whole delta chain from the base, which made a cold blame of
/// a long-history file several times slower.
fn horizon_odb(repo: &gix::Repository) -> gix::OdbHandle {
    let mut odb = repo.objects.clone();
    odb.unset_object_cache();
    odb.set_pack_cache(|| {
        Box::new(gix::odb::pack::cache::lru::StaticLinkedList::<64>::new(HORIZON_PACK_CACHE_BYTES))
    });
    odb
}

/// The id at `path` below `root`, descending only through trees, as [`crate::treepath::lookup`].
/// `memo` maps a tree below the root on `path` to the answer below it: most commits change
/// the root tree but share the subtree that holds `path`, so a walk decodes that once.
fn blob_at(
    odb: &gix::OdbHandle,
    root: gix::ObjectId,
    path: &str,
    buf: &mut Vec<u8>,
    memo: &mut HashMap<(usize, gix::ObjectId), Option<gix::ObjectId>>,
) -> Result<Option<gix::ObjectId>> {
    let parts: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    let mut id = root;
    let mut path_trees = Vec::new();
    let mut answer = None;
    for (depth, name) in parts.iter().enumerate() {
        if let Some(known) = memo.get(&(depth, id)) {
            answer = *known;
            break;
        }
        path_trees.push((depth, id));
        let Some(data) = gix::objs::Find::try_find(odb, &id, buf).map_err(git)? else {
            break;
        };
        if data.kind != gix::objs::Kind::Tree {
            break;
        }
        let tree = gix::objs::TreeRefIter::from_bytes(data.data, data.object_hash);
        let mut found = None;
        for entry in tree {
            let entry = entry.map_err(git)?;
            if entry.filename == name.as_bytes() {
                found = Some((entry.oid.to_owned(), entry.mode));
                break;
            }
        }
        let Some((oid, mode)) = found else {
            break;
        };
        if depth + 1 == parts.len() {
            answer = Some(oid);
            break;
        }
        if !mode.is_tree() {
            break;
        }
        id = oid;
    }
    // Root trees differ on almost every commit: keeping them would only grow the memo.
    for key in path_trees.into_iter().filter(|(depth, _)| *depth > 0) {
        memo.insert(key, answer);
    }
    Ok(answer)
}

#[cfg(test)]
mod tests;
