//! `limits.worktree_files` and `limits.worktree_diff_kb` reach the Uncommitted view.

use gitterrace_testkit::{Fixture, git};

use crate::{Repo, worktree_diff};

fn dirty_repo(p: &std::path::Path) -> Repo {
    let _ = Fixture::init(p).write("a.txt", "a\n").commit("init");
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
    assert_eq!(super::scoped_paths(&r, crate::Scope::All, 100).expect("paths").len(), 5);
    assert_eq!(super::scoped_paths(&r, crate::Scope::All, 2).expect("paths").len(), 2);
    assert!(super::is_dirty(&repo), "one staged file is dirty");
}

#[test]
fn worktree_diff_bytes_lists_larger_files_without_hunks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dirty_repo(dir.path());
    let hunks =
        |r: &Repo| worktree_diff(r).expect("diff").iter().filter(|f| !f.hunks.is_empty()).count();
    assert_eq!(hunks(&repo), 5);
    repo.set_worktree_limits(crate::WORKTREE_FILES, 1024);
    assert_eq!(hunks(&repo), 0, "1200-byte files over a 1 KiB limit keep no hunks");
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
        std::fs::write(p.join("old").join(format!("m{i}.txt")), format!("file {i}\nmore {i}\n"))
            .expect("write");
    }
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "init"]);
    git(p, &["mv", "old", "new"]);
    let repo = Repo::discover(p).expect("repo");
    repo.set_worktree_limits(10, crate::WORKTREE_DIFF_BYTES);
    let files = worktree_diff(&repo).expect("diff");
    assert_eq!(files.len(), 10, "{files:?}");
    assert!(files.iter().all(|f| f.kind == crate::ChangeKind::Renamed), "{files:?}");
}

/// Guard class: any scan that stops at `limits.worktree_files` must say so, whichever pass cut
/// it (the status walk or the staged pass) and for every consumer of the list. At the limit the
/// answer is "possibly incomplete", below it "complete".
#[test]
fn a_scan_at_the_limit_says_the_list_may_be_incomplete() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dirty_repo(dir.path());
    let omitted = |repo: &Repo| crate::worktree_listed(repo, crate::Scope::All).expect("list");
    repo.set_worktree_limits(6, crate::WORKTREE_DIFF_BYTES);
    assert!(!omitted(&repo).files_omitted, "five files under a limit of six are all of them");
    repo.set_worktree_limits(5, crate::WORKTREE_DIFF_BYTES);
    assert!(omitted(&repo).files_omitted, "at the limit more may exist");
    repo.set_worktree_limits(2, crate::WORKTREE_DIFF_BYTES);
    let l = omitted(&repo);
    assert!(l.files_omitted && l.files.len() == 2);
    let snap = crate::worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap");
    assert!(snap.files_omitted && snap.files.len() == 2, "a checkpoint is cut like the list");
}

#[test]
fn staged_files_cut_by_the_limit_say_so_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dirty_repo(dir.path());
    git(dir.path(), &["add", "."]);
    repo.set_worktree_limits(3, crate::WORKTREE_DIFF_BYTES);
    for scope in [crate::Scope::All, crate::Scope::Staged] {
        let l = crate::worktree_listed(&repo, scope).expect("list");
        assert!(l.files_omitted, "{scope:?}");
    }
    repo.set_worktree_limits(50, crate::WORKTREE_DIFF_BYTES);
    assert!(!crate::worktree_listed(&repo, crate::Scope::All).expect("list").files_omitted);
}

#[test]
fn a_since_comparison_carries_the_cut_of_the_checkpoint_and_of_the_scan_now() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dirty_repo(dir.path());
    let head = repo.resolve("HEAD").expect("head");
    let since = |repo: &Repo, before_omitted| {
        crate::worktree_diff_since(repo, &[], &head, before_omitted).expect("since").files_omitted
    };
    assert!(!since(&repo, false));
    assert!(since(&repo, true), "the checkpoint's own cut shows");
    repo.set_worktree_limits(2, crate::WORKTREE_DIFF_BYTES);
    assert!(since(&repo, false), "the scan now is cut");
}
