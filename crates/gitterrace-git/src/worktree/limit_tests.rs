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
fn worktree_files_caps_staged_files_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dirty_repo(dir.path());
    // Staged and unchanged since, so only the tree-to-index pass sees them.
    git(dir.path(), &["add", "."]);
    let r = super::open(&repo).expect("open");
    assert_eq!(super::changed_paths(&r, 100).expect("paths").len(), 5);
    assert_eq!(super::changed_paths(&r, 2).expect("paths").len(), 2);
    assert!(super::is_dirty(&repo), "one staged file is dirty");
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

/// A rename is two paths but one listed file: the cap must not cut the deleted sides off
/// (they sort after the new names) and leave the renames as delete plus add.
#[test]
fn worktree_files_counts_a_rename_as_one_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::create_dir(p.join("old")).expect("mkdir");
    for i in 0..10 {
        std::fs::write(
            p.join("old").join(format!("m{i}.txt")),
            format!("file {i}\nmore {i}\n"),
        )
        .expect("write");
    }
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "init"]);
    git(p, &["mv", "old", "new"]);
    let repo = Repo::discover(p).expect("repo");
    repo.set_worktree_limits(10, crate::WORKTREE_DIFF_BYTES);
    let files = worktree_diff(&repo).expect("diff");
    assert_eq!(files.len(), 10, "{files:?}");
    assert!(
        files.iter().all(|f| f.kind == crate::ChangeKind::Renamed),
        "{files:?}"
    );
}
