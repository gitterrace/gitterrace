//! The version stamp's dirty check covers the whole worktree, but not bench results.
#![expect(clippy::expect_used, reason = "test setup")]

#[path = "../stamp.rs"]
mod stamp;

use std::{fs, path::Path, process::Command};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(args)
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

/// A repo shaped like this workspace: a crate dir, a sibling crate and bench results.
fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path();
    for f in [
        "crates/app/build.rs",
        "crates/other/lib.rs",
        "bench/results/run.json",
    ] {
        let p = root.join(f);
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        fs::write(&p, "a").expect("write");
    }
    git(root, &["init", "-q"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "init"]);
    tmp
}

#[test]
fn a_change_in_another_crate_marks_the_build_dirty() {
    let tmp = repo();
    fs::write(tmp.path().join("crates/other/lib.rs"), "b").expect("write");
    assert!(stamp::is_dirty(&tmp.path().join("crates/app")));
}

#[test]
fn a_change_only_in_bench_results_stays_clean() {
    let tmp = repo();
    fs::write(tmp.path().join("bench/results/run.json"), "b").expect("write");
    fs::write(tmp.path().join("bench/results/new.json"), "c").expect("write");
    assert!(!stamp::is_dirty(&tmp.path().join("crates/app")));
}

#[test]
fn a_clean_tree_stays_clean() {
    let tmp = repo();
    assert!(!stamp::is_dirty(&tmp.path().join("crates/app")));
}
