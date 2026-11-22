//! Index-level equivalence: however the cache got to a history, it answers like a fresh build.
//! A record set is compared through its `Debug` form, sorted by commit id, which covers every
//! field a page reads.
#![expect(clippy::expect_used, reason = "test setup")]

use std::path::Path;

use gitterrace_git::Repo;
use gitterrace_index::Index;
use gitterrace_testkit::{Adversarial, git};

fn records(index: &Index, repo: &Path) -> Vec<String> {
    let source = Repo::discover(repo).expect("open");
    index.update(&source, "HEAD", &|_| {}).expect("update");
    let mut r: Vec<String> =
        index.records(&source, "HEAD").expect("records").iter().map(|r| format!("{r:?}")).collect();
    r.sort();
    r
}

fn fresh(repo: &Path) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tmp");
    records(&Index::open_dir(dir.path().join("c")).expect("index"), repo)
}

/// A shallow clone indexed, then unshallowed and indexed again in the same cache, equals a
/// fresh build of the full history. Without the shallow fingerprint in the manifest the old
/// boundary commit keeps its parentless diff (every file added) and this is red.
#[test]
fn an_unshallowed_clone_indexes_like_a_full_one() {
    // The fixture has a future-dated commit, which is pinned to the wall clock: freeze it.
    if !gitterrace_testkit::oracle::frozen("an_unshallowed_clone_indexes_like_a_full_one") {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let adv = Adversarial::build(dir.path());
    let cache = dir.path().join("cache");
    let index = Index::open_dir(cache.clone()).expect("index");
    let shallow = records(&index, &adv.shallow);
    let full = fresh(&adv.repo);
    assert_ne!(shallow, full, "the shallow fixture must differ, or this proves nothing");
    git(&adv.shallow, &["fetch", "-q", "--unshallow"]);
    let index = Index::open_dir(cache).expect("reopen");
    assert_eq!(records(&index, &adv.shallow), fresh(&adv.shallow));
    assert_eq!(fresh(&adv.shallow), full, "unshallowed history equals the original");
}
