//! Looking up a repo-relative path in a tree. Git paths split on `/` only. gix's
//! `lookup_entry_by_path` splits on the platform separator, so on Windows a `\` in a URL
//! would walk into a folder; every lookup goes through here instead.

use crate::error::{Result, git};

/// The entry at `path` (components split on `/`), or `None`. A `\` is part of a name.
pub(crate) fn lookup<'r>(
    tree: &gix::Tree<'r>,
    path: &str,
) -> Result<Option<gix::object::tree::Entry<'r>>> {
    tree.lookup_entry(path.split('/').filter(|c| !c.is_empty()).map(str::as_bytes))
        .map_err(git)
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
}
