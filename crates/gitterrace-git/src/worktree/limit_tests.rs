//! `limits.worktree_files` and `limits.worktree_diff_kb` reach the Uncommitted view.

use gitterrace_testkit::git;

use crate::{Repo, worktree_diff};

fn dirty_repo(p: &std::path::Path) -> Repo {
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "a\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "init"]);
    for i in 0..5 {
        std::fs::write(p.join(format!("n{i}.txt")), "x\n".repeat(600)).expect("write");
    }
    Repo::discover(p).expect("repo")
}

#[test]
fn worktree_files_caps_the_listed_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dirty_repo(dir.path());
    assert_eq!(worktree_diff(&repo).expect("diff").len(), 5);
    repo.set_worktree_limits(2, crate::WORKTREE_DIFF_BYTES);
    assert_eq!(worktree_diff(&repo).expect("diff").len(), 2);
}

#[test]
fn worktree_diff_bytes_lists_larger_files_without_hunks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dirty_repo(dir.path());
    let hunks = |r: &Repo| {
        worktree_diff(r)
            .expect("diff")
            .iter()
            .filter(|f| !f.hunks.is_empty())
            .count()
    };
    assert_eq!(hunks(&repo), 5);
    repo.set_worktree_limits(crate::WORKTREE_FILES, 1024);
    assert_eq!(
        hunks(&repo),
        0,
        "1200-byte files over a 1 KiB limit keep no hunks"
    );
}
