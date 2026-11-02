//! Looking up a repo-relative path in a tree. Git paths split on `/` only. gix's
//! `lookup_entry_by_path` splits on the platform separator, so on Windows a `\` in a URL
//! would walk into a folder; every lookup goes through here instead.

use crate::error::{Result, git};

/// The entry at `path` (components split on `/`), or `None`. A `\` is part of a name.
///
/// Only trees are descended into. A path below a submodule (a gitlink entry) or below a file
/// does not exist, as in git: gix would try to read the gitlink's commit as a tree, which is in
/// another repository, and fail with a missing object.
pub(crate) fn lookup<'r>(
    tree: &gix::Tree<'r>,
    path: &str,
) -> Result<Option<gix::object::tree::Entry<'r>>> {
    let mut parts = path.split('/').filter(|c| !c.is_empty()).peekable();
    let mut tree = tree.clone();
    while let Some(name) = parts.next() {
        let Some(entry) = tree.lookup_entry([name.as_bytes()]).map_err(git)? else {
            return Ok(None);
        };
        if parts.peek().is_none() {
            return Ok(Some(entry));
        }
        if !entry.mode().is_tree() {
            return Ok(None);
        }
        tree = entry.object().map_err(git)?.into_tree();
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git_at;

    use crate::Repo;

    /// A name holding a backslash is one component on every platform, and `a\b` is not `a/b`.
    #[test]
    fn backslash_is_a_name_not_a_separator() {
        let dir = tempfile::tempdir().expect("dir");
        let p = dir.path();
        std::fs::create_dir_all(p.join("a")).expect("mkdir");
        std::fs::write(p.join("a/b"), "x").expect("write");
        let d = "2024-01-01T00:00:00+00:00";
        git_at(p, d, &["init", "-q", "-b", "main"]);
        git_at(p, d, &["add", "a"]);
        git_at(p, d, &["commit", "-q", "-m", "m"]);
        let repo = Repo::discover(p).expect("repo");
        assert!(repo.blob("HEAD", "a/b").is_ok());
        assert!(repo.blob("HEAD", "a\\b").is_err());
        assert!(repo.tree("HEAD", "a\\").is_err());
    }

    /// A directory that was a submodule earlier (rust's `library/stdarch`) holds a gitlink in
    /// old trees, naming a commit of another repository. A path below it did not exist then:
    /// history and lookups must say so, not fail on the missing object. Seen red as "An
    /// object with id ... could not be found" with gix's multi-component lookup.
    #[test]
    fn a_gitlink_is_not_walked_into() {
        let dir = tempfile::tempdir().expect("dir");
        let p = dir.path();
        let d = "2024-01-01T00:00:00+00:00";
        git_at(p, d, &["init", "-q", "-b", "main"]);
        let gone = "1dfaa4db2479753a46a3e90f2c3c89d89d0b21f1";
        let info = format!("160000,{gone},sub");
        git_at(p, d, &["update-index", "--add", "--cacheinfo", &info]);
        git_at(p, d, &["commit", "-q", "-m", "submodule"]);
        git_at(p, d, &["rm", "-q", "--cached", "sub"]);
        std::fs::create_dir_all(p.join("sub")).expect("mkdir");
        std::fs::write(p.join("sub/f"), "x").expect("write");
        git_at(p, d, &["add", "sub"]);
        git_at(p, d, &["commit", "-q", "-m", "vendored"]);
        let repo = Repo::discover(p).expect("repo");
        let log = repo.log("HEAD", Some("sub/f"), 0, 10).expect("log");
        assert_eq!(log.len(), 1);
        assert!(repo.blob("HEAD~1", "sub/f").is_err());
    }
}
