//! The shared tree diff of a commit's files: one diff per commit and run, and none that outlives
//! its export run. Split from `tests` for size, with the helpers it needs.

use gitterrace_testkit::git_at;
use std::fs;
use std::sync::Arc;

use crate::repo::tests::{T0, commit, numbered, write};
use crate::{ChangeKind, Repo};

/// Two commits of 40 files each, the second editing all of them and renaming one.
fn wide_repo(d: &std::path::Path) {
    git_at(d, T0, &["init", "-q", "-b", "main"]);
    for i in 0..40 {
        write(d, &format!("f/{i:02}.txt"), &numbered(i + 1));
    }
    commit(d, "wide root");
    for i in 1..40 {
        write(d, &format!("f/{i:02}.txt"), &numbered(i + 3));
    }
    fs::create_dir(d.join("g")).expect("mkdir");
    fs::rename(d.join("f/00.txt"), d.join("g/renamed.txt")).expect("rename");
    commit(d, "wide edit with a rename");
}

/// Guard class: any read of a commit's files that redoes its tree diff. Under an export run's
/// memo, the commit page and every one of its files, in any order and repeated, run the
/// rename-detecting tree diff once per commit; red when a path bypasses the memo (a wide
/// commit then costs one full diff per file, quadratic in its width). The files read one by
/// one must also be the files of the page, so the memo cannot change an answer.
#[test]
fn a_commit_and_all_its_files_share_one_tree_diff() {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    wide_repo(d);
    let repo = Repo::discover(d).expect("repo");
    let memo = Arc::new(crate::ChangeMemo::new());
    repo.use_change_memo(&memo);
    let ids = repo.rev_list("HEAD").expect("rev_list");
    let diffs = || crate::diff::TREE_DIFFS.with(std::cell::Cell::get);
    let before = diffs();
    let mut renames = 0;
    for id in &ids {
        let page = repo.commit(id).expect("commit").files;
        assert!(page.len() >= 40, "{id}: {} files", page.len());
        renames += page
            .iter()
            .filter(|f| f.kind == ChangeKind::Renamed)
            .count();
        for round in 0..2 {
            for (n, want) in page.iter().enumerate().rev() {
                let got = repo.commit_file(id, n).expect("file").expect("in range");
                assert_eq!(
                    format!("{got:?}"),
                    format!("{want:?}"),
                    "{id} file {n} {round}"
                );
            }
        }
        assert!(
            repo.commit_file(id, page.len())
                .expect("past end")
                .is_none()
        );
    }
    assert_eq!(renames, 1, "the fixture's rename is detected");
    assert_eq!(diffs() - before, ids.len(), "one tree diff per commit");
}

/// Guard class: per-commit state a long-lived handle keeps outside its budgeted caches. A
/// handle without a run's memo (every server handle) keeps no tree diff between requests, and
/// once the run that owns a memo drops it, the handle that used it keeps none either. Red when
/// the memo lives on the handle: the second read is then a hit.
#[test]
fn no_tree_diff_outlives_its_export_run() {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    wide_repo(d);
    let repo = Repo::discover(d).expect("repo");
    let id = repo.resolve("HEAD").expect("head");
    let diffs = || crate::diff::TREE_DIFFS.with(std::cell::Cell::get);
    let read = || {
        let before = diffs();
        repo.commit_file(&id, 3).expect("file").expect("in range");
        diffs() - before
    };
    assert_eq!(
        (read(), read()),
        (1, 1),
        "a server handle keeps no tree diff"
    );
    let memo = Arc::new(crate::ChangeMemo::new());
    repo.use_change_memo(&memo);
    assert_eq!((read(), read()), (1, 0), "the run shares one");
    let weak = Arc::downgrade(&memo);
    drop(memo);
    assert!(weak.upgrade().is_none(), "the handle holds the run's memo");
    assert!(repo.change_memo().is_none());
    assert_eq!((read(), read()), (1, 1), "nothing kept after the run");
}

/// Guard class: a handle that loses the memo once one run has ended. Two export runs one after
/// the other on the same handle both share their tree diffs; red when the first run's ended
/// memo stays in place and the second run diffs on every read.
#[test]
fn a_reused_handle_shares_tree_diffs_in_every_run() {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    wide_repo(d);
    let repo = Repo::discover(d).expect("repo");
    let id = repo.resolve("HEAD").expect("head");
    let diffs = || crate::diff::TREE_DIFFS.with(std::cell::Cell::get);
    let read = || {
        let before = diffs();
        repo.commit_file(&id, 3).expect("file").expect("in range");
        diffs() - before
    };
    for run in 0..2 {
        let memo = Arc::new(crate::ChangeMemo::new());
        repo.use_change_memo(&memo);
        assert_eq!((read(), read()), (1, 0), "run {run} shares one tree diff");
    }
}
