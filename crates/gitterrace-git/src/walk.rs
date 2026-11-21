//! Bulk reads at a revision: file lists, blob scanning, commit scanning and per-entry last
//! commits. Each call opens its own repository handle so callers can run them on many threads.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::time::{Duration, Instant};

use gix::bstr::ByteSlice;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::error::git;
use crate::ident;
use crate::{CommitSummary, Repo, Result};

const OBJECT_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// The most tree entries, folders included, one walk of [`files`] records. A crafted tree with
/// millions of entries would otherwise hold them all before the filter; past this the list is
/// cut short, so the page shows a partial tree rather than exhaust memory. Not configurable: no
/// real repository comes near it.
const MAX_TREE_ENTRIES: usize = 500_000;

#[derive(Clone, Copy, Debug)]
pub struct BlobId(gix::ObjectId);

impl BlobId {
    pub fn hex(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct FileRef {
    pub path: String,
    pub blob: BlobId,
}

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

/// All regular files (not links or submodules) in the tree of `rev`, in tree order. Past
/// 500,000 tree entries the walk stops and the files found so far are the answer.
pub fn files(repo: &Repo, rev: &str) -> Result<Vec<FileRef>> {
    files_capped(repo, rev, MAX_TREE_ENTRIES)
}

fn files_capped(repo: &Repo, rev: &str, cap: usize) -> Result<Vec<FileRef>> {
    repo.tree_walks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let id = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let tree = r.find_commit(id).map_err(git)?.tree().map_err(git)?;
    let mut rec = Capped { rec: gix::traverse::tree::Recorder::default(), cap };
    // The cap stops the walk with a cancel: that ends it cleanly, anything else is an error.
    if let Err(e) = tree.traverse().breadthfirst(&mut rec)
        && rec.rec.records.len() < cap
    {
        return Err(git(e));
    }
    let mut out: Vec<FileRef> = rec
        .rec
        .records
        .into_iter()
        .filter(|e| e.mode.is_blob())
        .map(|e| FileRef { path: e.filepath.to_str_lossy().into_owned(), blob: BlobId(e.oid) })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    // The collect reuses the walk's buffer, sized for every entry with directories; callers
    // keep this list (the tree caches), so it holds the files alone.
    out.shrink_to_fit();
    Ok(out)
}

/// A [`gix::traverse::tree::Recorder`] that stops the walk at `cap` records.
struct Capped {
    rec: gix::traverse::tree::Recorder,
    cap: usize,
}

impl Capped {
    const fn room(
        &self,
        act: gix::traverse::tree::visit::Action,
    ) -> gix::traverse::tree::visit::Action {
        if self.rec.records.len() >= self.cap { std::ops::ControlFlow::Break(()) } else { act }
    }
}

impl gix::traverse::tree::Visit for Capped {
    fn pop_back_tracked_path_and_set_current(&mut self) {
        self.rec.pop_back_tracked_path_and_set_current();
    }
    fn pop_front_tracked_path_and_set_current(&mut self) {
        self.rec.pop_front_tracked_path_and_set_current();
    }
    fn push_back_tracked_path_component(&mut self, c: &gix::bstr::BStr) {
        self.rec.push_back_tracked_path_component(c);
    }
    fn push_path_component(&mut self, c: &gix::bstr::BStr) {
        self.rec.push_path_component(c);
    }
    fn pop_path_component(&mut self) {
        self.rec.pop_path_component();
    }
    fn visit_tree(
        &mut self,
        e: &gix::objs::tree::EntryRef<'_>,
    ) -> gix::traverse::tree::visit::Action {
        let act = self.rec.visit_tree(e);
        self.room(act)
    }
    fn visit_nontree(
        &mut self,
        e: &gix::objs::tree::EntryRef<'_>,
    ) -> gix::traverse::tree::visit::Action {
        let act = self.rec.visit_nontree(e);
        self.room(act)
    }
}

pub struct BlobReader {
    repo: gix::Repository,
}

impl BlobReader {
    pub fn open(repo: &Repo) -> Result<Self> {
        Ok(Self { repo: open(repo)? })
    }

    /// Content of a text blob, or `None` when it is larger than `max` bytes or binary.
    pub fn text(&self, id: BlobId, max: u64) -> Option<String> {
        String::from_utf8(self.data(id, max)?).ok()
    }

    /// The bytes of a blob no larger than `max` that is not binary.
    fn data(&self, id: BlobId, max: u64) -> Option<Vec<u8>> {
        if self.repo.find_header(id.0).ok()?.size() > max {
            return None;
        }
        let data = self.repo.find_object(id.0).ok()?.detach().data;
        (!crate::diff::is_binary(&data)).then_some(data)
    }

    /// Number of lines (a last line without newline counts), or `None` for binary content or
    /// blobs larger than `max` bytes.
    pub fn line_count(&self, id: BlobId, max: u64) -> Option<u32> {
        Some(crate::diff::line_count(&self.data(id, max)?))
    }
}

/// Object id of the tree at `path` in `rev`, a cheap cache key for directory listings.
pub fn tree_id(repo: &Repo, rev: &str, path: &str) -> Result<String> {
    let id = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let commit = r.find_commit(id).map_err(git)?;
    Ok(dir_oid(&commit, path.trim_matches('/'))?.map(|o| o.to_string()).unwrap_or_default())
}

/// Work done by one [`scan_commits`]: commits walked, and summaries built for the visitor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitScan {
    pub walked: usize,
    pub built: usize,
}

/// Visits commits reachable from `rev`, newest first.
///
/// `keep` sees the full message and the author as the mailmap names them, and only a kept
/// commit gets a summary built and passed to `f`, so a commit nobody wants costs no id,
/// parent or committer formatting. `f` returns `false` to stop.
pub fn scan_commits(
    repo: &Repo,
    rev: &str,
    mut keep: impl FnMut(&str, &str, &str) -> bool,
    mut f: impl FnMut(CommitSummary, &str) -> bool,
) -> Result<CommitScan> {
    let tip = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let mailmap = ident::mailmap(&r, tip);
    let walk = newest_first(&r, [tip])?;
    let mut scan = CommitScan::default();
    for info in walk {
        let commit = info.map_err(git)?.object().map_err(git)?;
        scan.walked += 1;
        repo.count_commit_read();
        let raw = commit.message_raw().map_err(git)?.to_str_lossy();
        let author = commit.author().map_err(git)?;
        let named = mailmap.resolve_cow(author);
        if !keep(&raw, &named.name.to_str_lossy(), &named.email.to_str_lossy()) {
            continue;
        }
        scan.built += 1;
        let message = raw.into_owned();
        let summary = crate::repo::summary(&commit, &mailmap)?;
        if !f(summary, &message) {
            break;
        }
    }
    Ok(scan)
}

/// Every commit reachable from `tips`, newest committer time first.
pub(crate) fn newest_first(
    repo: &gix::Repository,
    tips: impl IntoIterator<Item = impl Into<gix::ObjectId>>,
) -> Result<gix::revision::Walk<'_>> {
    repo.rev_walk(tips)
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
        .all()
        .map_err(git)
}

/// Commit ids reachable from `rev` in the canonical order: parents before children, ties among
/// ready commits broken by (committer time, oid) ascending.
///
/// The order is a function of the commit graph alone, so a build numbers commits the same
/// whatever the walk, the thread count or the build history. Parents outside the reachable set
/// (a shallow boundary) do not hold a commit back.
pub fn canonical_order(repo: &Repo, rev: &str) -> Result<Vec<String>> {
    let tip = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let walk = newest_first(&r, [tip])?;
    let mut nodes: Vec<(i64, gix::ObjectId, Vec<gix::ObjectId>)> = Vec::new();
    for info in walk {
        let info = info.map_err(git)?;
        let time = match info.commit_time {
            Some(t) => t,
            None => r.find_commit(info.id).map_err(git)?.time().map_err(git)?.seconds,
        };
        nodes.push((time, info.id, info.parent_ids.to_vec()));
    }
    Ok(topo_sort(&nodes).into_iter().map(|i| nodes[i].1.to_string()).collect())
}

/// Kahn's algorithm over `(time, id, parents)`, picking the smallest (time, id) among the ready
/// commits. Returns positions into `nodes`.
fn topo_sort(nodes: &[(i64, gix::ObjectId, Vec<gix::ObjectId>)]) -> Vec<usize> {
    let pos: HashMap<gix::ObjectId, usize> =
        nodes.iter().enumerate().map(|(i, n)| (n.1, i)).collect();
    let mut waiting = vec![0usize; nodes.len()];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, (_, _, parents)) in nodes.iter().enumerate() {
        // A parent listed twice still releases its child once per listing, so count each.
        for p in parents.iter().filter_map(|p| pos.get(p)) {
            waiting[i] += 1;
            children[*p].push(i);
        }
    }
    let mut ready: BinaryHeap<Reverse<(i64, gix::ObjectId, usize)>> = waiting
        .iter()
        .enumerate()
        .filter(|(_, w)| **w == 0)
        .map(|(i, _)| Reverse((nodes[i].0, nodes[i].1, i)))
        .collect();
    let mut out = Vec::with_capacity(nodes.len());
    while let Some(Reverse((_, _, i))) = ready.pop() {
        out.push(i);
        for &c in &children[i] {
            waiting[c] -= 1;
            if waiting[c] == 0 {
                ready.push(Reverse((nodes[c].0, nodes[c].1, c)));
            }
        }
    }
    out
}

fn dir_oid(commit: &gix::Commit<'_>, dir: &str) -> Result<Option<gix::ObjectId>> {
    let tree = commit.tree().map_err(git)?;
    if dir.is_empty() {
        return Ok(Some(tree.id));
    }
    Ok(crate::treepath::lookup(&tree, dir)?.filter(|e| e.mode().is_tree()).map(|e| e.object_id()))
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

/// A commit to visit, with the entry names it touches when the index named them.
type Step<'l> = Result<(gix::ObjectId, Option<&'l [String]>)>;

/// The commits [`last_commits_among`] visits: `among` when given, else the whole history.
fn steps<'l>(
    r: &'l gix::Repository,
    tip: gix::ObjectId,
    among: Option<&'l [(String, Vec<String>)]>,
) -> Result<Box<dyn Iterator<Item = Step<'l>> + 'l>> {
    Ok(match among {
        Some(list) => {
            Box::new(list.iter().map(|(id, names)| Ok((oid(id)?, Some(names.as_slice())))))
        }
        None => {
            Box::new(newest_first(r, [tip])?.map(|info| info.map(|i| (i.id, None)).map_err(git)))
        }
    })
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
    last_commits_among(repo, rev, dir, None, budget)
}

/// [`last_commits`], visiting only the commits in `among` when given.
///
/// `among` holds `(id, names)` for commits reachable from `rev` in the walk's order (newest commit
/// time first), where `names` are the entries of `dir` the commit's first-parent diff touches. It
/// must hold every commit whose first-parent diff touches something under `dir`; the index records
/// give that list. An entry is only ever resolved at a commit where it differs from the first
/// parent, so skipping a commit that touches no pending entry gives the same answer as the full
/// walk.
pub fn last_commits_among(
    repo: &Repo,
    rev: &str,
    dir: &str,
    among: Option<&[(String, Vec<String>)]>,
    budget: Duration,
) -> Result<(Vec<LastCommit>, bool)> {
    let start = Instant::now();
    let dir = dir.trim_matches('/');
    let tip = oid(&repo.resolve(rev)?)?;
    let r = open(repo)?;
    let tip_commit = r.find_commit(tip).map_err(git)?;
    // Ordered, so entries one commit resolves come out by name and not in a per-process
    // random hash order.
    let mut pending: std::collections::BTreeMap<_, _> =
        entries(&r, dir_oid(&tip_commit, dir)?).into_iter().collect();
    let ids = steps(&r, tip, among)?;
    let mut found: Vec<(Vec<u8>, gix::ObjectId)> = Vec::new();
    let mut complete = true;
    for step in ids {
        if pending.is_empty() {
            break;
        }
        let (id, names) = step?;
        if names.is_some_and(|ns| !ns.iter().any(|n| pending.contains_key(n.as_bytes()))) {
            continue;
        }
        if start.elapsed() >= budget {
            complete = false;
            break;
        }
        let commit = r.find_commit(id).map_err(git)?;
        let here = dir_oid(&commit, dir)?;
        // A shallow boundary lists parents it does not hold: it is a root here.
        let parents: Vec<_> = if crate::diff::is_shallow_boundary(&r, commit.id) {
            Vec::new()
        } else {
            commit.parent_ids().collect()
        };
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
            let subject = crate::repo::subject(c.message_raw().map_err(git)?);
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

#[cfg(test)]
mod tests {
    use super::topo_sort;

    /// A tree past the cap ends the walk with the files found so far, not an error.
    #[test]
    fn files_caps_entry_count_at_limit() {
        let dir = tempfile::tempdir().expect("tmp");
        let f = gitterrace_testkit::Fixture::init(dir.path())
            .write("a", "1\n")
            .write("b", "2\n")
            .write("d/c", "3\n")
            .commit("feat: files");
        let repo = crate::Repo::discover(f.path()).expect("repo");
        let all = super::files_capped(&repo, "HEAD", usize::MAX).expect("all");
        assert_eq!(all.len(), 3);
        let cut = super::files_capped(&repo, "HEAD", 2).expect("capped");
        assert!(!cut.is_empty() && cut.len() < 3, "{cut:?}");
    }

    fn id(n: u8) -> gix::ObjectId {
        gix::ObjectId::from_bytes_or_panic(&[n; 20])
    }

    /// A merged branch dated before commits already on main, a skewed clock (child older than
    /// its parent) and a tie on time: parents still come first, ties go by (time, oid).
    #[test]
    fn the_canonical_order_is_topological_then_time_then_oid() {
        // 1 root at 10. 2 on main at 50. 3 on a branch at 20. 4 skewed child of 2 at 5.
        // 5 and 6 tie at 60 as children of 4. 7 merges 6 and 3 at 70, 5 is a leaf.
        let nodes = vec![
            (70, id(7), vec![id(6), id(3)]),
            (60, id(6), vec![id(4)]),
            (60, id(5), vec![id(4)]),
            (5, id(4), vec![id(2)]),
            (50, id(2), vec![id(1)]),
            (20, id(3), vec![id(1)]),
            (10, id(1), vec![id(99)]),
        ];
        let got: Vec<u8> =
            topo_sort(&nodes).into_iter().map(|i| nodes[i].1.as_bytes()[0]).collect();
        assert_eq!(got, [1, 3, 2, 4, 5, 6, 7]);
    }

    fn order(nodes: &[(i64, gix::ObjectId, Vec<gix::ObjectId>)]) -> Vec<gix::ObjectId> {
        topo_sort(nodes).into_iter().map(|i| nodes[i].1).collect()
    }

    /// A merged branch commit dated before its parent still follows it.
    #[test]
    fn parents_come_first_even_when_younger() {
        let nodes = vec![
            (100, id(1), vec![]),
            (50, id(2), vec![id(1)]),
            (60, id(3), vec![]),
            (200, id(4), vec![id(2), id(3)]),
        ];
        assert_eq!(order(&nodes), [id(3), id(1), id(2), id(4)]);
    }

    /// Equal times break by oid, and the input order does not matter.
    #[test]
    fn ties_break_by_oid_and_input_order_does_not_matter() {
        let a = vec![(5, id(9), vec![]), (5, id(1), vec![])];
        let b = vec![(5, id(1), vec![]), (5, id(9), vec![])];
        assert_eq!(order(&a), order(&b));
        assert_eq!(order(&a)[0], id(1));
    }
}
