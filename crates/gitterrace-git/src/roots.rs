//! The root commits of a repository: every parentless commit its branches and tags reach.
//! Two repositories that share one are the same project (a fork, a clone), however their
//! folders are named. A shallow clone's boundary commit has parents it does not hold, so it is
//! no root: such a clone shares nothing until it is deepened.
//!
//! [`Roots`] keeps the set with the refs it was read at. Roots rarely change, so an update
//! walks only from tips it has not seen, hiding the ones it has; a deleted ref walks again
//! from scratch, as the roots it led to may be gone.

use std::collections::{BTreeSet, HashSet};
use std::hash::{Hash, Hasher};

use crate::error::git;
use crate::{Repo, Result};

/// The root commits of one repository, refreshed with [`Roots::update`].
#[derive(Debug, Clone, Default)]
pub struct Roots {
    /// Hash of every ref name and target the set was read at.
    key: Option<u64>,
    /// Ref names at that read, so a deletion is noticed.
    names: HashSet<String>,
    /// Commit tips already walked.
    tips: HashSet<gix::ObjectId>,
    ids: BTreeSet<String>,
}

impl Roots {
    /// Root commit ids, full hex, sorted.
    pub const fn ids(&self) -> &BTreeSet<String> {
        &self.ids
    }

    /// Whether `self` and `other` share a root commit.
    pub fn shares(&self, other: &Self) -> bool {
        self.ids.iter().any(|id| other.ids.contains(id))
    }

    /// Reads the refs and, when they moved since the last call, the roots behind the new
    /// tips. Returns whether anything was walked. Listing refs is cheap; a walk is paid once
    /// per ref change and covers only the commits not reached before.
    pub fn update(&mut self, repo: &Repo) -> Result<bool> {
        let r = repo.repo();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut names = HashSet::new();
        let mut tips = HashSet::new();
        let platform = r.references().map_err(git)?;
        for reference in platform.all().map_err(git)? {
            let mut reference = reference.map_err(git)?;
            let name = reference.name().as_bstr().to_string();
            // Branches, tags and remote branches: what the log lists. Stashes and notes are
            // not the project's history.
            if !["refs/heads/", "refs/tags/", "refs/remotes/"]
                .iter()
                .any(|p| name.starts_with(p))
            {
                continue;
            }
            name.hash(&mut h);
            match reference.target() {
                gix::refs::TargetRef::Object(id) => id.as_bytes().hash(&mut h),
                gix::refs::TargetRef::Symbolic(n) => n.as_bstr().hash(&mut h),
            }
            if let Ok(c) = reference.peel_to_commit() {
                tips.insert(c.id);
            }
            names.insert(name);
        }
        if let Ok(head) = r.head_id() {
            head.detach().as_bytes().hash(&mut h);
            tips.insert(head.detach());
        }
        let key = h.finish();
        if self.key == Some(key) {
            return Ok(false);
        }
        let dropped = self.names.iter().any(|n| !names.contains(n));
        if dropped {
            self.tips.clear();
            self.ids.clear();
        }
        let fresh: Vec<gix::ObjectId> = tips.difference(&self.tips).copied().collect();
        if !fresh.is_empty() {
            let shallow: HashSet<gix::ObjectId> = r
                .shallow_commits()
                .ok()
                .flatten()
                .map(|ids| ids.iter().copied().collect())
                .unwrap_or_default();
            let walk = r
                .rev_walk(fresh)
                .with_hidden(self.tips.iter().copied())
                .with_commit_graph(crate::graph::commit_graph(&r))
                .all()
                .map_err(git)?;
            for info in walk {
                let info = info.map_err(git)?;
                if info.parent_ids.is_empty() && !shallow.contains(&info.id) {
                    self.ids.insert(info.id.to_string());
                }
            }
        }
        self.tips = tips;
        self.names = names;
        self.key = Some(key);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gitterrace_testkit::git_at;

    use super::Roots;
    use crate::Repo;

    const T: &str = "2024-01-01T00:00:00+00:00";

    fn init(d: &Path) {
        git_at(d, T, &["init", "-q", "-b", "main"]);
        git_at(d, T, &["config", "user.name", "T"]);
        git_at(d, T, &["config", "user.email", "t@example.com"]);
    }

    fn commit(d: &Path, msg: &str) -> String {
        git_at(d, T, &["commit", "-q", "--allow-empty", "-m", msg]);
        git_at(d, T, &["rev-parse", "HEAD"])
    }

    fn roots(d: &Path) -> Roots {
        let mut r = Roots::default();
        r.update(&Repo::discover(d).expect("open")).expect("roots");
        r
    }

    fn clone(from: &Path, to: &Path, extra: &[&str]) {
        let url = format!("file://{}", from.display());
        let mut args = vec!["clone", "-q"];
        args.extend_from_slice(extra);
        let to_s = to.to_string_lossy().into_owned();
        args.extend([url.as_str(), to_s.as_str()]);
        git_at(from.parent().expect("parent"), T, &args);
    }

    /// A clone that grew its own commits shares the root; an unrelated repository does not.
    #[test]
    fn a_clone_shares_the_root_and_a_stranger_does_not() {
        let tmp = tempfile::tempdir().expect("tmp");
        let a = tmp.path().join("a");
        std::fs::create_dir(&a).expect("dir");
        init(&a);
        let root = commit(&a, "root");
        commit(&a, "two");
        let b = tmp.path().join("b");
        clone(&a, &b, &[]);
        git_at(&b, T, &["config", "user.name", "T"]);
        git_at(&b, T, &["config", "user.email", "t@example.com"]);
        commit(&b, "fork only");
        let c = tmp.path().join("c");
        std::fs::create_dir(&c).expect("dir");
        init(&c);
        commit(&c, "other root");
        let (ra, rb, rc) = (roots(&a), roots(&b), roots(&c));
        assert_eq!(ra.ids().iter().collect::<Vec<_>>(), [&root]);
        assert!(ra.shares(&rb) && rb.shares(&ra));
        assert!(!ra.shares(&rc) && !rc.shares(&rb), "disjoint roots");
    }

    /// An orphan branch adds a second root on the next update, and walks only the new tip;
    /// deleting it drops the root again. An unchanged ref set walks nothing.
    #[test]
    fn an_orphan_branch_adds_a_root_and_its_deletion_drops_it() {
        let tmp = tempfile::tempdir().expect("tmp");
        let d = tmp.path();
        init(d);
        let first = commit(d, "root");
        let repo = Repo::discover(d).expect("open");
        let mut r = Roots::default();
        assert!(r.update(&repo).expect("roots"));
        assert!(!r.update(&repo).expect("roots"), "same refs, no walk");
        git_at(d, T, &["checkout", "-q", "--orphan", "pages"]);
        let second = commit(d, "orphan root");
        assert!(r.update(&repo).expect("roots"));
        let want: std::collections::BTreeSet<_> = [first.clone(), second].into_iter().collect();
        assert_eq!(r.ids(), &want);
        git_at(d, T, &["checkout", "-q", "main"]);
        git_at(d, T, &["branch", "-q", "-D", "pages"]);
        assert!(r.update(&repo).expect("roots"));
        assert_eq!(r.ids().iter().collect::<Vec<_>>(), [&first]);
    }

    /// A `--depth 1` clone holds the tip but not the root: its boundary is no root, so it
    /// shares nothing with the full repository.
    #[test]
    fn a_shallow_clone_without_the_root_is_not_related() {
        let tmp = tempfile::tempdir().expect("tmp");
        let a = tmp.path().join("a");
        std::fs::create_dir(&a).expect("dir");
        init(&a);
        commit(&a, "root");
        commit(&a, "two");
        commit(&a, "three");
        let s = tmp.path().join("s");
        clone(&a, &s, &["--depth", "1"]);
        let (ra, rs) = (roots(&a), roots(&s));
        assert!(rs.ids().is_empty(), "{:?}", rs.ids());
        assert!(!ra.shares(&rs));
    }
}
