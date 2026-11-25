use gitterrace_testkit::{Fixture, git};

use super::{BIG_HASH, SinceDiff, SnapFile, content_id, worktree_diff_since, worktree_snapshot};
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
    let snap = worktree_snapshot(&repo, 50, u64::MAX).expect("snap").files;
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
    let snap = worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap").files;
    assert!(snap.iter().all(|f| f.path != "secret.txt"));
}

#[test]
fn since_lists_only_what_changed_after_the_checkpoint() {
    let (d, repo) = repo();
    let p = d.path();
    std::fs::write(p.join("a.txt"), "1\nx\n").expect("write");
    std::fs::write(p.join("b.txt"), "B\n").expect("write");
    let snap = worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap").files;
    std::fs::write(p.join("a.txt"), "1\nx\ny\n").expect("write");
    let d = since(&repo, &snap);
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
    let snap = worktree_snapshot(&repo, 0, u64::MAX).expect("snap").files;
    assert!(snap[0].text.is_none());
    let same = since(&repo, &snap);
    assert!(same.files.is_empty() && same.unchanged == 1 && same.gone.is_empty());
    std::fs::write(p.join("a.txt"), "1\nz\n").expect("write");
    let moved = since(&repo, &snap);
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
    let snap: Vec<SnapFile> = worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap").files;
    std::fs::write(p.join("a.txt"), "1\n2\n").expect("write");
    let d = since(&repo, &snap);
    assert_eq!(d.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
}

#[test]
fn a_snapshot_stops_copying_at_its_budget_but_still_hashes_every_file() {
    let (d, repo) = repo();
    let p = d.path();
    for i in 0..5 {
        std::fs::write(p.join(format!("n{i}.txt")), "z".repeat(100)).expect("write");
    }
    let snap = worktree_snapshot(&repo, 1 << 20, 250).expect("snap").files;
    let n: Vec<&SnapFile> = snap.iter().filter(|f| f.path.starts_with('n')).collect();
    assert_eq!(n.len(), 5, "every path stays");
    assert!(n.iter().all(|f| f.hash == content_id(&[b'z'; 100])));
    assert_eq!(n.iter().filter(|f| f.text.is_some()).count(), 2);
}

/// The comparison a checkpoint taken at the current HEAD gets.
fn since(repo: &Repo, snap: &[SnapFile]) -> SinceDiff {
    since_at(repo, snap, &repo.resolve("HEAD").expect("head"))
}

fn since_at(repo: &Repo, snap: &[SnapFile], head: &str) -> SinceDiff {
    worktree_diff_since(repo, snap, head, false).expect("since")
}

fn paths(d: &SinceDiff) -> Vec<&str> {
    d.files.iter().map(|f| f.path.as_str()).collect()
}

/// Guard class: any path the checkpoint did not list that changes through a HEAD move
/// (commit, reset, checkout) after it is taken. It reads the checkpoint's own HEAD as the
/// baseline, so the page cannot say "nothing changed" for a file only the new commit touched.
#[test]
fn a_clean_file_committed_after_the_checkpoint_shows_as_changed() {
    let (d, repo) = repo();
    let p = d.path();
    let at = repo.resolve("HEAD").expect("head");
    let snap = worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap").files;
    assert_eq!(snap.len(), 0);
    std::fs::write(p.join("a.txt"), "1\n2\n3\n").expect("write");
    git(p, &["commit", "-qam", "more"]);
    let d = since_at(&repo, &snap, &at);
    assert_eq!(paths(&d), ["a.txt"]);
    assert_eq!(d.files[0].additions, 1);
    assert!(!d.baseline_missing);
}

#[test]
fn additions_deletions_and_renames_committed_since_the_checkpoint_show_up() {
    let (d, repo) = repo();
    let p = d.path();
    let at = repo.resolve("HEAD").expect("head");
    let snap = worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap").files;
    std::fs::write(p.join("new.txt"), "n\n").expect("write");
    git(p, &["rm", "-q", "b.txt"]);
    git(p, &["mv", "a.txt", "c.txt"]);
    git(p, &["add", "-A"]);
    git(p, &["commit", "-qm", "shuffle"]);
    let d = since_at(&repo, &snap, &at);
    assert_eq!(paths(&d), ["a.txt", "b.txt", "c.txt", "new.txt"]);
}

#[test]
fn a_reset_back_to_the_checkpoint_head_shows_nothing_changed() {
    let (d, repo) = repo();
    let p = d.path();
    let at = repo.resolve("HEAD").expect("head");
    let snap = worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap").files;
    std::fs::write(p.join("a.txt"), "x\n").expect("write");
    git(p, &["commit", "-qam", "more"]);
    git(p, &["reset", "-q", "--hard", &at]);
    let d = since_at(&repo, &snap, &at);
    assert_eq!(d.files.len(), 0);
}

#[test]
fn a_reset_to_an_older_commit_shows_what_the_commits_since_added() {
    let (d, repo) = repo();
    let p = d.path();
    let first = repo.resolve("HEAD").expect("head");
    std::fs::write(p.join("a.txt"), "second\n").expect("write");
    git(p, &["commit", "-qam", "second"]);
    let second = repo.resolve("HEAD").expect("head");
    let snap = worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap").files;
    git(p, &["reset", "-q", "--hard", &first]);
    let d = since_at(&repo, &snap, &second);
    assert_eq!(paths(&d), ["a.txt"], "the file went back to what the first commit held");
}

#[test]
fn a_checkpoint_head_missing_from_the_repository_says_so() {
    let (_d, repo) = repo();
    let snap = Vec::new();
    let d = since_at(&repo, &snap, &"ab".repeat(20));
    assert!(d.baseline_missing);
}
