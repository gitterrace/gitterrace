//! Last commit per directory entry over histories with more than one root.
#![expect(clippy::expect_used, reason = "test setup")]

use std::collections::HashMap;
use std::time::Duration;

use gitterrace_git::{Repo, last_commits};
use gitterrace_testkit::{Adversarial, Fixture};

const AMPLE: Duration = Duration::from_secs(60);

fn by_name(repo: &Repo, rev: &str, dir: &str) -> (HashMap<String, String>, bool) {
    let (found, complete) = last_commits(repo, rev, dir, AMPLE).expect("last commits");
    let map = found.into_iter().map(|c| (c.name, c.id)).collect();
    (map, complete)
}

#[test]
fn each_entry_goes_to_the_root_that_holds_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(tmp.path())
        .at("2024-01-01T12:00:00Z")
        .write("lib/x.rs", "x\n")
        .commit("feat: old root");
    let old = f.git(&["rev-parse", "HEAD"]);
    f.git(&["checkout", "-q", "--orphan", "other"]);
    f.git(&["rm", "-rq", "--cached", "."]);
    std::fs::remove_dir_all(tmp.path().join("lib")).expect("rm lib");
    let f = f.at("2024-02-01T12:00:00Z").write("a.txt", "a\n").commit("feat: new root");
    let new = f.git(&["rev-parse", "HEAD"]);
    let f = f.at("2024-03-01T12:00:00Z");
    f.git(&["merge", "-q", "--allow-unrelated-histories", "-m", "merge", "main"]);
    let repo = Repo::discover(tmp.path()).expect("open");
    let (got, complete) = by_name(&repo, "HEAD", "");
    assert!(complete);
    assert_eq!(got.get("lib"), Some(&old), "lib only ever lived in the old root");
    assert_eq!(got.get("a.txt"), Some(&new));
}

#[test]
fn octopus_with_a_second_root_credits_its_file_to_that_root() {
    let tmp = tempfile::tempdir().expect("tmp");
    let fx = Adversarial::build(tmp.path());
    let repo = Repo::discover(&fx.repo).expect("open");
    let (got, complete) = by_name(&repo, &fx.tip, "");
    assert!(complete);
    assert_eq!(got.get("o.txt"), Some(&fx.roots[1]));
    assert_eq!(got.get("a.txt"), Some(&fx.tip));
    assert_eq!(got.get(".mailmap"), Some(&fx.mailmap));
    assert_ne!(got.get("s.txt"), Some(&fx.roots[0]));
    assert_ne!(got.get("s.txt"), Some(&fx.roots[1]));
}

#[test]
fn a_walk_cut_by_its_budget_says_it_is_not_final() {
    let tmp = tempfile::tempdir().expect("tmp");
    let fx = Adversarial::build(tmp.path());
    let repo = Repo::discover(&fx.repo).expect("open");
    let (found, complete) = last_commits(&repo, &fx.tip, "", Duration::ZERO).expect("walk");
    assert!(!complete, "a zero budget cannot finish the walk");
    assert!(found.is_empty());
}
