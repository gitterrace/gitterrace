use gitterrace_testkit::{Fixture, git};

use super::{BIG_HASH, SnapFile, content_id, worktree_diff_since, worktree_snapshot};
use crate::Repo;

fn repo() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    let _ = Fixture::init(dir.path()).write("a.txt", "1\n2\n").write("b.txt", "b\n").commit("init");
    let repo = Repo::discover(dir.path()).expect("repo");
    (dir, repo)
}

#[test]
fn a_snapshot_copies_small_text_and_hashes_the_rest() {
    let (d, repo) = repo();
    let p = d.path();
    std::fs::write(p.join("a.txt"), "1\nx\n").expect("write");
    std::fs::write(p.join("bin.dat"), b"\0\x01\x02").expect("write");
    std::fs::write(p.join("big.txt"), "y".repeat(100)).expect("write");
    std::fs::remove_file(p.join("b.txt")).expect("rm");
    let snap = worktree_snapshot(&repo, 50).expect("snap");
    let by = |n: &str| snap.iter().find(|f| f.path == n).expect(n);
    assert_eq!(by("a.txt").text.as_deref(), Some(&b"1\nx\n"[..]));
    assert_eq!(by("a.txt").hash, content_id(b"1\nx\n"));
    assert!(by("bin.dat").text.is_none() && !by("bin.dat").hash.is_empty());
    assert!(by("big.txt").text.is_none() && by("big.txt").hash == content_id(&[b'y'; 100]));
    assert!(by("b.txt").hash.is_empty() && by("b.txt").text.is_none());
    assert_ne!(BIG_HASH, "");
}

#[test]
fn an_ignored_file_is_never_in_a_snapshot() {
    let (d, repo) = repo();
    let p = d.path();
    std::fs::write(p.join(".gitignore"), "secret.txt\n").expect("write");
    std::fs::write(p.join("secret.txt"), "key\n").expect("write");
    let snap = worktree_snapshot(&repo, 1 << 20).expect("snap");
    assert!(snap.iter().all(|f| f.path != "secret.txt"));
}

#[test]
fn since_lists_only_what_changed_after_the_checkpoint() {
    let (d, repo) = repo();
    let p = d.path();
    std::fs::write(p.join("a.txt"), "1\nx\n").expect("write");
    std::fs::write(p.join("b.txt"), "B\n").expect("write");
    let snap = worktree_snapshot(&repo, 1 << 20).expect("snap");
    std::fs::write(p.join("a.txt"), "1\nx\ny\n").expect("write");
    let d = worktree_diff_since(&repo, &snap).expect("since");
    assert_eq!(d.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    assert_eq!((d.files[0].additions, d.files[0].deletions), (1, 0));
    assert_eq!(d.unchanged, 1);
    assert_eq!(d.gone.len(), 0);
}

#[test]
fn since_a_copy_that_is_gone_diffs_against_head_and_says_so() {
    let (d, repo) = repo();
    let p = d.path();
    std::fs::write(p.join("a.txt"), "1\nx\n").expect("write");
    let snap = worktree_snapshot(&repo, 0).expect("snap");
    assert!(snap[0].text.is_none());
    let same = worktree_diff_since(&repo, &snap).expect("since");
    assert!(same.files.is_empty() && same.unchanged == 1 && same.gone.is_empty());
    std::fs::write(p.join("a.txt"), "1\nz\n").expect("write");
    let moved = worktree_diff_since(&repo, &snap).expect("since");
    assert_eq!(moved.gone, ["a.txt"]);
    assert_eq!(moved.files.len(), 1);
    assert_eq!(moved.files[0].additions, 1);
    git(p, &["status", "-s"]);
}

#[test]
fn a_file_reverted_to_head_since_the_checkpoint_shows_as_changed() {
    let (d, repo) = repo();
    let p = d.path();
    std::fs::write(p.join("a.txt"), "1\nx\n").expect("write");
    let snap: Vec<SnapFile> = worktree_snapshot(&repo, 1 << 20).expect("snap");
    std::fs::write(p.join("a.txt"), "1\n2\n").expect("write");
    let d = worktree_diff_since(&repo, &snap).expect("since");
    assert_eq!(d.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
}
