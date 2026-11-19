//! The best common ancestor of two revisions.

use crate::Result;
use crate::error::git;
use crate::repo::Repo;

impl Repo {
    /// The best common ancestor of `a` and `b` (what `git merge-base` prints), or `None` when
    /// the histories share no commit.
    pub fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        let repo = self.repo();
        let (a, b) = (self.resolve_id(&repo, a)?, self.resolve_id(&repo, b)?);
        let base = repo.merge_base(a, b).map_err(git)?;
        Ok(base.map(|id| id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::Fixture;

    /// The merge base is the commit a branch left from, not the tip the other side moved to,
    /// and unrelated histories have none. Catches a base that follows either tip.
    #[test]
    fn merge_base_is_where_the_branch_left_and_none_for_unrelated_histories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = Fixture::init(dir.path()).write("a", "1\n").commit("root");
        let root = f.git(&["rev-parse", "HEAD"]);
        let f = f.branch("side").write("b", "1\n").commit("side");
        let f = f.checkout("main").write("c", "1\n").commit("main");
        f.git(&["checkout", "-q", "--orphan", "other"]);
        f.git(&["commit", "-q", "--allow-empty", "-m", "orphan"]);
        let repo = crate::Repo::discover(f.path()).expect("open");
        assert_eq!(
            repo.merge_base("main", "side").expect("base").as_deref(),
            Some(root.trim())
        );
        assert_eq!(repo.merge_base("main", "other").expect("none"), None);
        assert!(repo.merge_base("main", "nope").is_err());
    }
}
