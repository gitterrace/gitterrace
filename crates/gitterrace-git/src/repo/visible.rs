//! Which commits a public server may show: those reachable from a branch, tag,
//! remote-tracking ref or HEAD. A full id names any object in the store, a stash or a dangling
//! commit included, so a committed-only handle checks it here before answering.
//!
//! The reachable set is walked once and kept for one key, the raw targets of the public roots.
//! A moved, new or deleted root changes the key. When the roots only grew, the walk covers the
//! new history alone, otherwise the set is rebuilt: a deleted ref may have hidden commits.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use crate::Result;
use crate::error::git;

/// Ref prefixes whose targets are public roots, the same ones `committed_rev` resolves.
const PUBLIC_PREFIXES: [&str; 3] = ["refs/heads/", "refs/tags/", "refs/remotes/"];

/// The reachable set of one root key. Empty and unbuilt until a raw id asks.
#[derive(Default)]
pub(crate) struct Visible {
    key: Option<u64>,
    roots: HashSet<gix::ObjectId>,
    reachable: HashSet<gix::ObjectId>,
    built: bool,
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
                self.reachable.clear();
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
        Ok(self.reachable.contains(&id))
    }

    /// Commits held, so a test can see the set is built once.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.reachable.len()
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
        let name = reference.name().as_bstr();
        if !PUBLIC_PREFIXES
            .iter()
            .any(|p| name.starts_with(p.as_bytes()))
        {
            continue;
        }
        name.hash(&mut h);
        match reference.target() {
            gix::refs::TargetRef::Object(id) => id.as_bytes().hash(&mut h),
            gix::refs::TargetRef::Symbolic(n) => n.as_bstr().hash(&mut h),
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
        let name = reference.name().as_bstr();
        if PUBLIC_PREFIXES
            .iter()
            .any(|p| name.starts_with(p.as_bytes()))
            && let Ok(c) = reference.peel_to_commit()
        {
            out.insert(c.id);
        }
    }
    out
}

/// Adds every commit reachable from `tips` but not from `hidden` to `into`.
fn walk_into(
    repo: &gix::Repository,
    tips: Vec<gix::ObjectId>,
    hidden: Vec<gix::ObjectId>,
    into: &mut HashSet<gix::ObjectId>,
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
        into.insert(info.map_err(git)?.id);
    }
    Ok(())
}
