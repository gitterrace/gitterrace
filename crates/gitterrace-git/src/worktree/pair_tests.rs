use std::path::Path;

use gitterrace_testkit::git;

use crate::worktree::held;
use crate::{ChangeKind, Repo, worktree_diff, worktree_file_diff};

fn init(p: &Path, files: &[(&str, &str)]) {
    git(p, &["init", "-q", "-b", "main"]);
    for (n, c) in files {
        if let Some(d) = p.join(n).parent() {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        std::fs::write(p.join(n), c).expect("write");
    }
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "init"]);
}

fn kinds(repo: &Repo) -> Vec<(String, ChangeKind, Option<String>)> {
    worktree_diff(repo)
        .expect("diff")
        .into_iter()
        .map(|f| (f.path, f.kind, f.old_path))
        .collect()
}

/// A `git mv` (staged or not) is one pure rename; an edited move stays delete plus add.
#[test]
fn an_exact_move_is_one_rename() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init(p, &[("a.txt", "same\n"), ("b.txt", "old\n")]);
    git(p, &["mv", "a.txt", "moved.txt"]);
    std::fs::rename(p.join("b.txt"), p.join("c.txt")).expect("mv");
    std::fs::write(p.join("c.txt"), "new\n").expect("write");
    let repo = Repo::discover(p).expect("repo");
    let d = worktree_diff(&repo).expect("diff");
    assert_eq!(
        kinds(&repo),
        [
            ("b.txt".into(), ChangeKind::Deleted, None),
            ("c.txt".into(), ChangeKind::Added, None),
            (
                "moved.txt".into(),
                ChangeKind::Renamed,
                Some("a.txt".into())
            ),
        ]
    );
    assert!(d[2].hunks.is_empty() && d[2].additions == 0);
}

/// Three identical deletes and two identical adds: the first two sources pair in path order,
/// the third stays a deletion. Empty files never pair, however many match.
#[test]
fn identical_files_pair_in_order_and_empty_files_never() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init(
        p,
        &[
            ("d1", "x\n"),
            ("d2", "x\n"),
            ("d3", "x\n"),
            ("e1", ""),
            ("e2", ""),
        ],
    );
    for n in ["d1", "d2", "d3", "e1", "e2"] {
        std::fs::remove_file(p.join(n)).expect("rm");
    }
    for n in ["n1", "n2"] {
        std::fs::write(p.join(n), "x\n").expect("write");
    }
    std::fs::write(p.join("n3"), "").expect("write");
    let repo = Repo::discover(p).expect("repo");
    assert_eq!(
        kinds(&repo),
        [
            ("d3".into(), ChangeKind::Deleted, None),
            ("e1".into(), ChangeKind::Deleted, None),
            ("e2".into(), ChangeKind::Deleted, None),
            ("n1".into(), ChangeKind::Renamed, Some("d1".into())),
            ("n2".into(), ChangeKind::Renamed, Some("d2".into())),
            ("n3".into(), ChangeKind::Added, None),
        ]
    );
}

/// The fragment route answers for a changed file only: never an ignored file, anything
/// under `.git`, or a path that climbs with `..`, though each of them exists on disk.
#[test]
fn one_file_reads_only_a_changed_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init(
        p,
        &[
            (".gitignore", "*.env\n"),
            ("a.txt", "1\n"),
            ("d/k.txt", "k\n"),
        ],
    );
    std::fs::write(p.join("a.txt"), "2\n").expect("write");
    std::fs::write(p.join("prod.env"), "SECRET=1\n").expect("write");
    std::fs::write(p.join("untracked.txt"), "u\n").expect("write");
    let repo = Repo::discover(p).expect("repo");
    let one = worktree_file_diff(&repo, "a.txt")
        .expect("read")
        .expect("changed");
    assert_eq!((one.kind, one.additions), (ChangeKind::Modified, 1));
    assert!(
        worktree_file_diff(&repo, "untracked.txt")
            .expect("read")
            .is_some()
    );
    for bad in [
        "prod.env",
        ".git/config",
        ".git/HEAD",
        "d/../untracked.txt",
        "./a.txt",
        "d//k.txt",
        "d/k.txt",
        "nope.txt",
    ] {
        assert!(
            worktree_file_diff(&repo, bad).expect("read").is_none(),
            "{bad}"
        );
    }
}

/// Guard class: the view holds one file's bytes at a time, not the whole change. 30 modified
/// files of 64 KiB each and 30 moved ones: the peak is one file's old and new side.
#[test]
fn bytes_held_stay_one_file_wide() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let body = |i: usize| format!("{i:03}\n").repeat(16 * 1024);
    let files: Vec<(String, String)> = (0..60).map(|i| (format!("f{i:02}"), body(i))).collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    init(p, &refs);
    for (i, (n, _)) in files.iter().enumerate() {
        if i < 30 {
            std::fs::write(p.join(n), body(i + 100)).expect("write");
        } else {
            std::fs::rename(p.join(n), p.join(format!("m{i:02}"))).expect("mv");
        }
    }
    let repo = Repo::discover(p).expect("repo");
    held::take_peak();
    let d = worktree_diff(&repo).expect("diff");
    let peak = held::take_peak();
    assert_eq!(d.len(), 60);
    assert_eq!(
        d.iter().filter(|f| f.kind == ChangeKind::Renamed).count(),
        30
    );
    let one = body(0).len();
    assert!(
        peak <= 3 * one,
        "{peak} bytes held at once, one file is {one}"
    );
}

fn scoped(repo: &Repo, scope: crate::Scope) -> Vec<(String, u32, u32)> {
    crate::worktree_diff_scoped(repo, scope)
        .expect("diff")
        .into_iter()
        .map(|f| (f.path, f.additions, f.deletions))
        .collect()
}

/// Staged-only, unstaged-only and both-changed files each show in the right scope, with the
/// hunks of that scope's pair of states; untracked counts as unstaged; counts agree.
#[test]
fn each_scope_lists_its_files_with_its_own_hunks() {
    use crate::Scope;
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init(p, &[("s.txt", "a\n"), ("u.txt", "a\n"), ("b.txt", "a\n")]);
    std::fs::write(p.join("s.txt"), "a\nstaged\n").expect("w");
    std::fs::write(p.join("b.txt"), "a\nstaged\n").expect("w");
    git(p, &["add", "s.txt", "b.txt"]);
    std::fs::write(p.join("u.txt"), "a\nunstaged\n").expect("w");
    std::fs::write(p.join("b.txt"), "a\nstaged\nmore\n").expect("w");
    std::fs::write(p.join("new.txt"), "n\n").expect("w");
    let repo = Repo::discover(p).expect("repo");
    assert_eq!(
        scoped(&repo, Scope::Staged),
        [("b.txt".into(), 1, 0), ("s.txt".into(), 1, 0)]
    );
    assert_eq!(
        scoped(&repo, Scope::Unstaged),
        [
            ("b.txt".into(), 1, 0),
            ("new.txt".into(), 1, 0),
            ("u.txt".into(), 1, 0)
        ]
    );
    assert_eq!(
        scoped(&repo, Scope::All),
        [
            ("b.txt".into(), 2, 0),
            ("new.txt".into(), 1, 0),
            ("s.txt".into(), 1, 0),
            ("u.txt".into(), 1, 0)
        ]
    );
    let c = crate::worktree_counts(&repo).expect("counts");
    assert_eq!((c.all, c.staged, c.unstaged), (4, 2, 3));
    assert!(
        crate::worktree_file_diff_scoped(&repo, "u.txt", Scope::Staged)
            .expect("read")
            .is_none()
    );
}
