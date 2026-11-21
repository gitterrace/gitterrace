//! Which commits a public server may show: those reachable from a branch, tag,
//! remote-tracking ref or HEAD. A full id names any object in the store, a stash or a dangling
//! commit included, so a committed-only handle checks it here before answering.
//!
//! The reachable set is walked once and kept for one key, the raw targets of the public roots.
//! A moved, new or deleted root changes the key. When the roots only grew, the walk covers the
//! new history alone, otherwise the set is rebuilt: a deleted ref may have hidden commits.
//! The set holds the first 8 bytes of each id. A prefix hit is only an answer when the object
//! store holds no other object with that 16-hex prefix: then the reachable commit it came from
//! is the asked id itself. A shared prefix is decided by a fresh walk that keeps nothing.
//! An idle server drops the set ([`crate::Repo::clear_visible`]) and the next raw id walks
//! again.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use crate::Result;
use crate::error::git;
use crate::repo::{hash_ref, is_public_ref};

/// The reachable set of one root key. Empty and unbuilt until a raw id asks.
#[derive(Default)]
pub(crate) struct Visible {
    key: Option<u64>,
    roots: HashSet<gix::ObjectId>,
    /// Sorted id prefixes: 8 bytes a commit, 10 MB on a 1.3M-commit history, where full ids
    /// cost 20 and a hash set three to four times that.
    reachable: Vec<u64>,
    built: bool,
    /// Test seam: treat every prefix as shared, so a hit always takes the exact walk.
    #[cfg(test)]
    ambiguous: bool,
}

impl Visible {
    /// Whether `id` is reachable from a public root of `repo`. Walks history at most once per
    /// root change; a walk error leaves the set unbuilt and refuses.
    pub(crate) fn contains(&mut self, repo: &gix::Repository, id: gix::ObjectId) -> Result<bool> {
        let key = root_key(repo)?;
        if self.key != Some(key) {
            let roots = public_roots(repo);
            if self.built && roots.is_superset(&self.roots) {
                let fresh: Vec<_> = roots.difference(&self.roots).copied().collect();
                let old: Vec<_> = self.roots.iter().copied().collect();
                self.built = false;
                walk_into(repo, fresh, old, &mut self.reachable)?;
                self.built = true;
            } else {
                self.reachable = Vec::new();
                self.built = false;
            }
            self.roots = roots;
            self.key = Some(key);
        }
        if self.roots.contains(&id) {
            return Ok(true);
        }
        if !self.built {
            self.reachable.clear();
            let roots: Vec<_> = self.roots.iter().copied().collect();
            walk_into(repo, roots, Vec::new(), &mut self.reachable)?;
            self.built = true;
        }
        if self.reachable.binary_search(&prefix(&id)).is_err() {
            return Ok(false);
        }
        #[cfg(test)]
        let shared = self.ambiguous;
        #[cfg(not(test))]
        let shared = false;
        if !shared && unique_prefix(repo, id) {
            return Ok(true);
        }
        let roots: Vec<_> = self.roots.iter().copied().collect();
        walk_has(repo, roots, id)
    }

    /// Drops the set and its key, returning whether it held commits. The next raw id walks
    /// again, so a server that nobody browses keeps no per-commit state here.
    pub(crate) fn clear(&mut self) -> bool {
        let held = !self.reachable.is_empty();
        *self = Self::default();
        held
    }

    /// Commits held, so a test can see the set is built once.
    #[cfg(test)]
    pub(crate) const fn len(&self) -> usize {
        self.reachable.len()
    }

    /// Adds `id`'s prefix as a colliding reachable commit would, and marks every prefix shared.
    #[cfg(test)]
    pub(crate) fn inject_collision(&mut self, id: gix::ObjectId) {
        let p = prefix(&id);
        if let Err(i) = self.reachable.binary_search(&p) {
            self.reachable.insert(i, p);
        }
        self.ambiguous = true;
    }

    /// Heap bytes the set holds.
    #[cfg(test)]
    pub(crate) const fn heap_bytes(&self) -> usize {
        self.reachable.capacity() * size_of::<u64>()
    }
}

/// A hash of every public ref's name and raw target plus HEAD's commit. Cheap: nothing is
/// peeled, so it runs per request.
fn root_key(repo: &gix::Repository) -> Result<u64> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Ok(head) = repo.head_id() {
        head.as_bytes().hash(&mut h);
    }
    for reference in repo.references().map_err(git)?.all().map_err(git)? {
        let reference = reference.map_err(git)?;
        if is_public_ref(reference.name().as_bstr()) {
            hash_ref(&reference, &mut h);
        }
    }
    Ok(h.finish())
}

/// The commits the public refs and HEAD peel to. A ref that does not peel to a commit (a
/// tagged tree, a broken ref) adds nothing.
fn public_roots(repo: &gix::Repository) -> HashSet<gix::ObjectId> {
    let mut out = HashSet::new();
    if let Ok(c) = repo.head_commit() {
        out.insert(c.id);
    }
    let Ok(platform) = repo.references() else {
        return out;
    };
    let Ok(all) = platform.all() else {
        return out;
    };
    for mut reference in all.flatten() {
        if is_public_ref(reference.name().as_bstr())
            && let Ok(c) = reference.peel_to_commit()
        {
            out.insert(c.id);
        }
    }
    out
}

/// Adds every commit reachable from `tips` but not from `hidden` to the sorted `into`, which
/// stays sorted, deduplicated and trimmed to its length.
fn walk_into(
    repo: &gix::Repository,
    tips: Vec<gix::ObjectId>,
    hidden: Vec<gix::ObjectId>,
    into: &mut Vec<u64>,
) -> Result<()> {
    if tips.is_empty() {
        return Ok(());
    }
    let walk = repo
        .rev_walk(tips)
        .with_hidden(hidden)
        .with_commit_graph(crate::graph::commit_graph(repo))
        .all()
        .map_err(git)?;
    for info in walk {
        into.push(prefix(&info.map_err(git)?.id));
    }
    into.sort_unstable();
    into.dedup();
    into.shrink_to_fit();
    Ok(())
}

/// The first 8 bytes of `id`, big-endian so the order matches the ids'.
fn prefix(id: &gix::oid) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&id.as_bytes()[..8]);
    u64::from_be_bytes(b)
}

/// Whether `id` is the only object in the store whose id starts with its 16-hex prefix. An
/// error answers no, which sends the caller to the exact walk.
fn unique_prefix(repo: &gix::Repository, id: gix::ObjectId) -> bool {
    let Ok(p) = gix::hash::Prefix::new(&id, 16) else {
        return false;
    };
    matches!(repo.objects.lookup_prefix(p, None), Ok(Some(Ok(found))) if found == id)
}

/// Whether `id` is reachable from `tips`, by a walk that keeps nothing.
fn walk_has(repo: &gix::Repository, tips: Vec<gix::ObjectId>, id: gix::ObjectId) -> Result<bool> {
    if tips.is_empty() {
        return Ok(false);
    }
    let walk = repo
        .rev_walk(tips)
        .with_commit_graph(crate::graph::commit_graph(repo))
        .all()
        .map_err(git)?;
    for info in walk {
        if info.map_err(git)?.id == id {
            return Ok(true);
        }
    }
    Ok(false)
}
