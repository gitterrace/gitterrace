use gitterrace_testkit::git;

use crate::{ChangeKind, Repo, worktree_diff};

#[test]
fn a_bare_repository_is_never_dirty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src");
    std::fs::create_dir(&src).expect("mkdir");
    git(&src, &["init", "-q", "-b", "main"]);
    std::fs::write(src.join("a.txt"), "1\n").expect("write");
    git(&src, &["add", "."]);
    git(&src, &["commit", "-qm", "init"]);
    let bare = dir.path().join("bare.git");
    git(dir.path(), &["clone", "-q", "--bare", "src", bare.to_str().expect("utf8")]);
    let repo = Repo::discover(&bare).expect("repo");
    assert!(!crate::is_dirty(&repo));
    assert_eq!(crate::worktree_signature(&repo).expect("sig"), 0);
    assert!(worktree_diff(&repo).expect("diff").is_empty());
}

#[test]
fn shows_staged_unstaged_untracked_and_deleted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    for (n, c) in [("a.txt", "1\n2\n3\n"), ("b.txt", "x\n"), ("c.txt", "y\n")] {
        std::fs::write(p.join(n), c).expect("write");
    }
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "init"]);
    let repo = Repo::discover(p).expect("repo");
    assert!(!crate::is_dirty(&repo));
    std::fs::write(p.join("a.txt"), "1\ntwo\n3\n").expect("write");
    std::fs::write(p.join("b.txt"), "staged\n").expect("write");
    git(p, &["add", "b.txt"]);
    std::fs::remove_file(p.join("c.txt")).expect("rm");
    std::fs::write(p.join("new.txt"), "n\n").expect("write");
    assert!(crate::is_dirty(&repo));
    let d = worktree_diff(&repo).expect("diff");
    let kind = |n: &str| d.iter().find(|f| f.path == n).map(|f| f.kind);
    assert_eq!(kind("a.txt"), Some(ChangeKind::Modified));
    assert_eq!(kind("b.txt"), Some(ChangeKind::Modified));
    assert_eq!(kind("c.txt"), Some(ChangeKind::Deleted));
    assert_eq!(kind("new.txt"), Some(ChangeKind::Added));
    let a = d.iter().find(|f| f.path == "a.txt").expect("a");
    assert_eq!((a.additions, a.deletions), (1, 1));
    assert!(!p.join(".git/index.lock").exists());
}

#[test]
fn signature_moves_on_edit_and_new_file_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "1\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "init"]);
    let repo = Repo::discover(p).expect("repo");
    let sig = || crate::worktree_signature(&repo).expect("sig");
    let clean = sig();
    assert_eq!(sig(), clean, "an untouched tree is stable");
    std::fs::write(p.join("a.txt"), "changed\n").expect("write");
    let edited = sig();
    assert_ne!(edited, clean, "editing a tracked file");
    std::fs::write(p.join("a.txt"), "changed again\n").expect("write");
    let again = sig();
    assert_ne!(again, edited, "editing a modified file again");
    std::fs::write(p.join("new.txt"), "n\n").expect("write");
    assert_ne!(sig(), again, "a new untracked file");
}

#[test]
#[cfg(unix)]
fn large_file_listed_without_reading_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "a\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "init"]);
    // Unreadable, so any read fails and the file would be skipped: listing it proves
    // the size cap ran on metadata alone.
    let big = p.join("dump.sql");
    std::fs::File::create(&big)
        .and_then(|f| f.set_len(2 * super::WORKTREE_DIFF_BYTES))
        .expect("sparse");
    std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let d = worktree_diff(&Repo::discover(p).expect("repo")).expect("diff");
    let f = d.iter().find(|f| f.path == "dump.sql").expect("listed");
    assert!(f.binary && f.hunks.is_empty());
}

#[test]
#[cfg(unix)]
fn symlinked_directory_is_not_followed() {
    let outside = tempfile::tempdir().expect("tempdir");
    std::fs::write(outside.path().join("secret.txt"), "secret\n").expect("write");
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::create_dir(p.join("docs")).expect("dir");
    std::fs::write(p.join("docs/secret.txt"), "public\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "init"]);
    std::fs::remove_dir_all(p.join("docs")).expect("rm");
    std::os::unix::fs::symlink(outside.path(), p.join("docs")).expect("link");
    let d = worktree_diff(&Repo::discover(p).expect("repo")).expect("diff");
    let leaked =
        d.iter().flat_map(|f| &f.hunks).flat_map(|h| &h.lines).any(|l| l.text.contains("secret"));
    assert!(!leaked, "{d:?}");
}
