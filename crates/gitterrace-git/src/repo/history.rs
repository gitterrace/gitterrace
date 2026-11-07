//! Commit history: logs, path-limited walks, summaries and the index's numstat stream.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use gix::bstr::ByteSlice;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::error::git;
use crate::ident;
use crate::repo::{
    INDEX_WORKER_CACHE_BYTES, INDEX_WORKER_PACK_CACHE_BYTES, NUMSTAT_MEMO_PAIRS, NUMSTAT_WINDOW,
    Repo,
};
use crate::{CommitDetail, CommitSummary, Error, MAX_TEXT_BYTES, Result};

impl Repo {
    /// Commits reachable from `rev`, newest first, optionally limited to `path`.
    pub fn log(
        &self,
        rev: &str,
        path: Option<&str>,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<CommitSummary>> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let path = path.map(|p| p.trim_matches('/')).filter(|p| !p.is_empty());
        let walk = repo
            .rev_walk([tip])
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .all()
            .map_err(git)?;
        let mailmap = ident::mailmap(&repo, tip);
        let mut out = Vec::new();
        let mut seen = 0;
        if let Some(path) = path {
            // The walk pops in output order, so it can stop once the page is full.
            let ids = simplified(&repo, tip, path, None, Some(skip.saturating_add(limit)))?;
            for id in ids.into_iter().skip(skip).take(limit) {
                out.push(summary(&repo.find_commit(id).map_err(git)?, &mailmap)?);
            }
            return Ok(out);
        }
        for info in walk {
            if out.len() >= limit {
                break;
            }
            let info = info.map_err(git)?;
            let commit = info.object().map_err(git)?;
            if seen < skip {
                seen += 1;
                continue;
            }
            out.push(summary(&commit, &mailmap)?);
        }
        Ok(out)
    }

    /// The first `limit` commits [`Self::log`] lists without a path, in its order, as
    /// `(id, parents)`. Reads commit headers only, so the graph can look past the page for the
    /// parents its lines run to at little cost.
    pub fn parents(&self, rev: &str, limit: usize) -> Result<Vec<(String, Vec<String>)>> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let walk = repo
            .rev_walk([tip])
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .all()
            .map_err(git)?;
        let mut out = Vec::new();
        for info in walk.take(limit) {
            let info = info.map_err(git)?;
            let parents = info.parent_ids.iter().map(ToString::to_string).collect();
            out.push((info.id.to_string(), parents));
        }
        Ok(out)
    }

    /// The ids [`Self::log`] with `path` lists, in its order, from `history`: every commit
    /// reachable from the tip (tip first) as `(id, parents, touched)`. `touched` must be true
    /// for every commit whose first-parent diff touches `path` (the index knows). The others
    /// hold `path` as their first parent does, so they follow it unlisted with only a commit
    /// header read: tree lookups are paid by the touched commits alone.
    /// The bool is false when a commit could not be read; the list is then empty and the
    /// caller walks as [`Self::log`] does, failing at the same place.
    pub fn touching(&self, path: &str, history: &[(&str, &[String], bool)]) -> (Vec<String>, bool) {
        let repo = self.repo();
        let path = path.trim_matches('/');
        let known = || -> Result<(gix::ObjectId, HashMap<gix::ObjectId, bool>)> {
            let mut touched = HashMap::with_capacity(history.len());
            for (id, _, t) in history {
                touched.insert(self.full_id(&repo, id)?, *t);
            }
            let (tip, _, _) = history
                .first()
                .ok_or_else(|| Error::Git("empty history".into()))?;
            Ok((self.full_id(&repo, tip)?, touched))
        };
        let listed =
            known().and_then(|(tip, touched)| simplified(&repo, tip, path, Some(&touched), None));
        listed.map_or_else(
            |_| (Vec::new(), false),
            |ids| (ids.iter().map(ToString::to_string).collect(), true),
        )
    }

    /// Summaries of exactly these commits, in order, mapped through the `.mailmap` at the tip
    /// of `rev`, as [`Self::log`] maps them.
    pub fn summaries_at(&self, rev: &str, ids: &[String]) -> Result<Vec<CommitSummary>> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let mailmap = ident::mailmap(&repo, tip);
        ids.iter()
            .map(|id| {
                summary(
                    &repo.find_commit(self.full_id(&repo, id)?).map_err(git)?,
                    &mailmap,
                )
            })
            .collect()
    }

    /// Author times of the commits reachable from `rev` with a commit time at or after `since`,
    /// newest first, and the oldest of them when the whole history is in the window. Reads only
    /// commit headers, so a busy history stays cheap. The history is whole when the cutoff
    /// dropped no parent: every parent named was walked. A root met inside the window is not
    /// enough, a young orphan branch merged in is a root too. `None` when the history reaches
    /// past `since`.
    pub fn commit_times_since(&self, rev: &str, since: i64) -> Result<(Vec<i64>, Option<i64>)> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let walk = repo
            .rev_walk([tip])
            .sorting(Sorting::ByCommitTimeCutoff {
                order: CommitTimeOrder::NewestFirst,
                seconds: since,
            })
            .all()
            .map_err(git)?;
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut parents = Vec::new();
        for info in walk {
            let info = info.map_err(git)?;
            seen.insert(info.id);
            parents.extend(info.parent_ids.iter().copied());
            out.push(info.object().map_err(git)?.author().map_err(git)?.seconds());
        }
        let whole = parents.iter().all(|p| seen.contains(p));
        let first = if whole {
            out.iter().copied().min()
        } else {
            None
        };
        Ok((out, first))
    }

    /// Commit ids reachable from `rev`, oldest first. Used by the indexer.
    pub fn rev_list(&self, rev: &str) -> Result<Vec<String>> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
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

    /// Commit ids reachable from `head` but not from `base` (`base..head`), newest first, at
    /// most `limit` of them. Reads commit headers only, and stops at the limit, so a branch the
    /// index never walked still has its commits and a huge range costs `limit` headers.
    pub fn range(&self, base: &str, head: &str, limit: usize) -> Result<Vec<String>> {
        let repo = self.repo();
        let (b, h) = (self.resolve_id(&repo, base)?, self.resolve_id(&repo, head)?);
        let walk = repo
            .rev_walk([h])
            .with_hidden([b])
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .all()
            .map_err(git)?;
        let mut ids = Vec::new();
        for info in walk.take(limit) {
            ids.push(info.map_err(git)?.id.to_string());
        }
        Ok(ids)
    }

    /// One commit with its diff against the first parent.
    pub fn commit(&self, id: &str) -> Result<CommitDetail> {
        let repo = self.repo();
        let oid = self.resolve_id(&repo, id)?;
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
            files: crate::diff::files_of(
                &repo,
                &self.raw_changes(&repo, &commit)?,
                self.text_limit(),
            )?,
        })
    }

    /// One file of [`Self::commit`]'s list, by index, diffing only that file's blobs. The tree
    /// diff is shared with the commit page and the commit's other files.
    pub fn commit_file(&self, id: &str, index: usize) -> Result<Option<crate::FileDiff>> {
        let repo = self.repo();
        let oid = self.resolve_id(&repo, id)?;
        let commit = repo.find_commit(oid).map_err(git)?;
        let raw = self.raw_changes(&repo, &commit)?;
        crate::diff::change_at(&repo, &raw, index, self.text_limit())
    }

    /// The commit's tree diff against its first parent, through the run's change memo if one
    /// is in force.
    fn raw_changes(
        &self,
        repo: &gix::Repository,
        commit: &gix::Commit<'_>,
    ) -> Result<std::sync::Arc<Vec<gix::object::tree::diff::ChangeDetached>>> {
        let diff = || {
            let mut cache = crate::diff::resource_cache(repo)?;
            crate::diff::raw_changes(repo, commit, &mut cache)
        };
        self.change_memo().map_or_else(
            || diff().map(std::sync::Arc::new),
            |memo| memo.get(commit.id, diff),
        )
    }

    /// Summaries of exactly these commits, in order. Each commit is mapped through its own
    /// `.mailmap`, parsed once per distinct blob instead of once per commit.
    pub fn summaries(&self, ids: &[String]) -> Result<Vec<CommitSummary>> {
        let repo = self.repo();
        let mut maps: std::collections::HashMap<Option<gix::ObjectId>, gix::mailmap::Snapshot> =
            std::collections::HashMap::new();
        ids.iter()
            .map(|id| {
                let oid = self.full_id(&repo, id)?;
                let commit = repo.find_commit(oid).map_err(git)?;
                let blob = ident::mailmap_blob(&repo, oid);
                let map = maps
                    .entry(blob)
                    .or_insert_with(|| ident::parse_mailmap(&repo, blob));
                summary(&commit, map)
            })
            .collect()
    }

    /// Summaries of exactly these commits, in order, with identities as written in the commit.
    /// The indexer stores these, so a later `.mailmap` applies to cached history without a
    /// rebuild, see [`Self::map_identities`].
    pub fn raw_summaries(&self, ids: &[String]) -> Result<Vec<CommitSummary>> {
        let repo = self.repo();
        let none = gix::mailmap::Snapshot::default();
        ids.iter()
            .map(|id| {
                summary(
                    &repo.find_commit(self.full_id(&repo, id)?).map_err(git)?,
                    &none,
                )
            })
            .collect()
    }

    /// `(name, email)` pairs mapped through the `.mailmap` at the tip of `rev`, in order. The
    /// tip's file applies to all of history, as in git and in [`Self::log`].
    pub fn map_identities(
        &self,
        rev: &str,
        pairs: &[(String, String)],
    ) -> Result<Vec<(String, String)>> {
        let repo = self.repo();
        let map = ident::mailmap(&repo, self.resolve_id(&repo, rev)?);
        Ok(pairs
            .iter()
            .map(|(name, email)| {
                let sig = gix::actor::SignatureRef {
                    name: name.as_bytes().as_bstr(),
                    email: email.as_bytes().as_bstr(),
                    time: "",
                };
                let got = map.resolve(sig);
                (
                    got.name.to_str_lossy().into_owned(),
                    got.email.to_str_lossy().into_owned(),
                )
            })
            .collect())
    }

    /// Ids in the shallow file: commits whose parents this clone does not have. Diffs treat
    /// them as roots.
    pub fn shallow_ids(&self) -> Vec<String> {
        self.repo()
            .shallow_commits()
            .ok()
            .flatten()
            .map(|ids| ids.iter().map(ToString::to_string).collect())
            .unwrap_or_default()
    }

    /// Per-file line counts against the first parent, without hunk text, for many commits
    /// spread over all cores. Results are in the order of `ids`; the work is CPU bound and
    /// independent, so the indexer batches it.
    /// A commit whose objects cannot be read (a pruned or hostile object store) is `None`
    /// instead of failing the whole batch, so the caller can leave it out and retry it later.
    /// A shallow clone's boundary is diffed as a root.
    pub fn numstat_many(&self, ids: &[String]) -> Result<Vec<Option<Vec<crate::FileDiff>>>> {
        let mut out = Vec::with_capacity(ids.len());
        self.numstat_stream(ids, |_, commit| {
            out.push(commit.map(|(_, files)| files));
            Ok(())
        })?;
        Ok(out)
    }

    /// [`Self::numstat_many`] with each commit's raw summary (identities as written), handed to
    /// `emit` in the order of `ids` as soon as it and every earlier one are done. Workers read
    /// and diff commits on all cores with no batch barrier, and run at most a bounded window
    /// ahead of `emit`, so memory stays flat however long the list is.
    pub fn numstat_stream(
        &self,
        ids: &[String],
        emit: impl FnMut(usize, Option<(CommitSummary, Vec<crate::FileDiff>)>) -> Result<()>,
    ) -> Result<()> {
        let workers = match self.workers {
            0 => std::thread::available_parallelism().map_or(1, usize::from),
            n => n,
        };
        let none = gix::mailmap::Snapshot::default();
        let memo = crate::diff::CountMemo::new(NUMSTAT_MEMO_PAIRS);
        crate::diff::stream(
            ids,
            workers,
            NUMSTAT_WINDOW,
            || {
                let mut repo = self.repo();
                // Ten workers each keeping the shared 128 MB cache cost over 1 GB; the
                // diffs of one commit touch few objects twice.
                repo.object_cache_size(INDEX_WORKER_CACHE_BYTES);
                repo.objects.set_pack_cache(|| {
                    Box::new(gix::odb::pack::cache::lru::StaticLinkedList::<64>::new(
                        INDEX_WORKER_PACK_CACHE_BYTES,
                    ))
                });
                let cache = crate::diff::resource_cache(&repo)?;
                Ok((repo, cache))
            },
            |(repo, cache), id| {
                let oid = self.full_id(repo, id)?;
                let commit = repo.find_commit(oid).map_err(git)?;
                let Ok(files) = crate::diff::changes_with(
                    repo,
                    &commit,
                    false,
                    MAX_TEXT_BYTES,
                    cache,
                    Some(&memo),
                ) else {
                    return Ok(None);
                };
                Ok(Some((summary(&commit, &none)?, files)))
            },
            |r| r.as_ref().map_or(0, |(s, files)| result_bytes(s, files)),
            emit,
        )
    }
}

/// Heap bytes one index result holds, roughly: what the reorder window budgets.
fn result_bytes(s: &CommitSummary, files: &[crate::FileDiff]) -> usize {
    let fixed = std::mem::size_of::<crate::FileDiff>();
    let files: usize = files
        .iter()
        .map(|f| fixed + f.path.len() + f.old_path.as_ref().map_or(0, String::len))
        .sum();
    files + s.subject.len() + s.parents.len() * 48 + 256
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

/// The commits `git log <tip> -- path` lists, in its order. Like git, the walk queues only the
/// parents [`simplify`] keeps and pops the newest commit date first (equal dates in queue
/// order), so a hidden side branch never shows, and a skewed old date on a followed side
/// still comes when it is reached. `touched` (from the index) marks the commits whose
/// first-parent diff touches `path`; the others hold `path` as their first parent does, so
/// they follow it unlisted without reading any tree. With `stop`, the walk ends once that many
/// are listed: it pops in output order, so they are exactly the first `stop` of the full list.
fn simplified(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    path: &str,
    touched: Option<&HashMap<gix::ObjectId, bool>>,
    stop: Option<usize>,
) -> Result<Vec<gix::ObjectId>> {
    let mut out = Vec::new();
    let mut queued = std::collections::HashSet::from([tip]);
    let mut queue = std::collections::BinaryHeap::new();
    let mut order = 0u64;
    let time = |id: gix::ObjectId| -> Result<i64> {
        let commit = repo.find_commit(id).map_err(git)?;
        Ok(commit.time().map_err(git)?.seconds)
    };
    queue.push((time(tip)?, Reverse(order), tip));
    while let Some((_, _, id)) = queue.pop() {
        let commit = repo.find_commit(id).map_err(git)?;
        let (hit, follow) = match touched.and_then(|t| t.get(&id)) {
            Some(false) if !crate::diff::is_shallow_boundary(repo, id) => {
                let first = commit.parent_ids().next().map(gix::Id::detach);
                (false, first.into_iter().collect())
            }
            _ => simplify(repo, &commit, path)?,
        };
        if hit {
            out.push(id);
            if stop.is_some_and(|n| out.len() >= n) {
                break;
            }
        }
        for parent in follow {
            if queued.insert(parent) {
                order += 1;
                queue.push((time(parent)?, Reverse(order), parent));
            }
        }
    }
    Ok(out)
}

/// Git's default history simplification for `git log -- path`, one commit: whether it is
/// listed, and the parents the walk goes on to. A commit whose path equals a parent's (TREESAME)
/// is not listed and follows only the first such parent, so a merge that took one side hides
/// the other side's history. Otherwise it is listed and every parent is followed. A root (or a
/// shallow boundary) is listed when the path exists.
fn simplify(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    path: &str,
) -> Result<(bool, Vec<gix::ObjectId>)> {
    let here = path_id(commit, path)?;
    let parents: Vec<gix::ObjectId> = commit.parent_ids().map(gix::Id::detach).collect();
    if parents.is_empty() || crate::diff::is_shallow_boundary(repo, commit.id) {
        return Ok((here.is_some(), Vec::new()));
    }
    for parent in &parents {
        let pc = repo.find_commit(*parent).map_err(git)?;
        if path_id(&pc, path)? == here {
            return Ok((false, vec![*parent]));
        }
    }
    Ok((true, parents))
}

fn path_id(commit: &gix::Commit<'_>, path: &str) -> Result<Option<gix::ObjectId>> {
    let tree = commit.tree().map_err(git)?;
    let entry = crate::treepath::lookup(&tree, path)?;
    Ok(entry.map(|e| e.object_id()))
}
