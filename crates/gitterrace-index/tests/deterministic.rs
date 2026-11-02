//! The stored index is a pure function of the repository: built with one worker, four or ten,
//! and twice with ten, every cache file is byte for byte the same. Pins that scheduling (the
//! work queue, the reorder window, which worker takes which commit) never shows in the output.
#![expect(clippy::expect_used, reason = "test setup")]

use std::collections::BTreeMap;
use std::path::Path;

use gitterrace_git::Repo;
use gitterrace_index::Index;
use gitterrace_testkit::{Adversarial, Fixture};

/// Builds a fresh index of `repo` with `threads` workers, returns every file in it.
fn build(repo: &Path, threads: usize) -> BTreeMap<String, Vec<u8>> {
    let source = Repo::discover(repo).expect("open").with_workers(threads);
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("gitterrace");
    let index = Index::open_dir(cache.clone()).expect("index");
    index.update(&source, "HEAD", &|_| {}).expect("update");
    std::fs::read_dir(&cache)
        .expect("read cache")
        .map(|e| {
            let e = e.expect("entry");
            let name = e.file_name().to_string_lossy().into_owned();
            (name, std::fs::read(e.path()).expect("read"))
        })
        .collect()
}

fn assert_same_on_any_thread_count(repo: &Path) {
    let one = build(repo, 1);
    assert!(
        one.get("commits.jsonl").is_some_and(|c| !c.is_empty()),
        "nothing was indexed"
    );
    for threads in [4, 10, 10] {
        let got = build(repo, threads);
        assert_eq!(
            got.keys().collect::<Vec<_>>(),
            one.keys().collect::<Vec<_>>()
        );
        for (name, body) in &one {
            assert!(
                got[name] == *body,
                "{name} differs between 1 and {threads} threads"
            );
        }
    }
}

/// Many commits of very uneven cost (a mass rename among small edits), so the workers finish
/// out of order and the reorder window is exercised.
#[test]
fn many_uneven_commits_index_the_same_on_any_thread_count() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut f = Fixture::init(dir.path()).at("2024-01-01T00:00:00Z");
    for i in 0..60 {
        let body = (0..40).fold(String::new(), |b, l| b + &format!("line {l} of {i}\n"));
        f = f.write(&format!("src/f{i}.rs"), &body);
    }
    f = f.commit("feat: start");
    for i in 0..120 {
        if i == 30 {
            f.git(&["mv", "src", "lib"]);
            f = f.commit("refactor: move everything");
            continue;
        }
        let top = if i < 30 { "src" } else { "lib" };
        f = f
            .write(&format!("{top}/f{}.rs", i % 60), &format!("edit {i}\n"))
            .commit(&format!("fix(core): edit {i}"));
    }
    assert_same_on_any_thread_count(f.path());
}

/// Two roots, an octopus merge, absurd dates and a shallow clone.
#[test]
fn the_adversarial_repository_indexes_the_same_on_any_thread_count() {
    let dir = tempfile::tempdir().expect("tempdir");
    let adv = Adversarial::build(dir.path());
    assert_same_on_any_thread_count(&adv.repo);
    assert_same_on_any_thread_count(&adv.shallow);
}

/// Opt-in, for a real history: `TEST_DETERMINISM_REPO=/path/to/git.git`.
#[test]
fn a_real_repository_indexes_the_same_on_any_thread_count() {
    if let Some(repo) = std::env::var_os("TEST_DETERMINISM_REPO") {
        assert_same_on_any_thread_count(Path::new(&repo));
    }
}

/// Built at HEAD~k, updated to HEAD, then merged: every cache file byte for byte the one-step
/// build's, because the merge renumbers strings and ordinals by first use in canonical order.
#[test]
#[ignore = "waits until the index builds through the segment merge"]
fn two_steps_then_merge_equal_one_step_byte_for_byte() {
    panic!("not wired yet");
}
