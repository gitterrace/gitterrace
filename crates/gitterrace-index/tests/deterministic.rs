//! The stored index is a pure function of the repository: built with one worker, four or ten,
//! and twice with ten, every cache file is byte for byte the same. Pins that scheduling (the
//! work queue, the reorder window, which worker takes which commit) never shows in the output.
#![expect(clippy::expect_used, reason = "test setup")]

use std::collections::BTreeMap;
use std::path::Path;

use gitterrace_git::Repo;
use gitterrace_index::Index;
use gitterrace_index::seg::format::{Kind, section};
use gitterrace_index::seg::manifest::Manifest;
use gitterrace_index::seg::read::Store;
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
        one.get("MANIFEST").is_some_and(|c| !c.is_empty()),
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

/// Every live file of the cache in `dir`, in manifest order, with its kind. Names carry the
/// generation that wrote them, so only kind and bytes are compared.
fn live_files(dir: &Path) -> Vec<(u8, Vec<u8>)> {
    let m = Manifest::load(dir)
        .expect("manifest")
        .expect("a segment set");
    m.live
        .iter()
        .map(|f| (f.kind, std::fs::read(dir.join(&f.name)).expect("read live")))
        .collect()
}

/// The kinds the manifest names, in order, after checking each named file exists.
fn kinds(dir: &Path) -> Vec<u8> {
    let m = Manifest::load(dir)
        .expect("manifest")
        .expect("a segment set");
    for f in &m.live {
        assert!(dir.join(&f.name).is_file(), "{} is named but gone", f.name);
    }
    m.live.iter().map(|f| f.kind).collect()
}

fn segments(dir: &Path) -> usize {
    Manifest::load(dir).ok().flatten().map_or(0, |m| {
        m.live
            .iter()
            .filter(|f| f.kind == Kind::Segment as u8)
            .count()
    })
}

/// Built at HEAD~k, updated to HEAD, then merged: every cache file byte for byte the one-step
/// build's, because the merge renumbers strings and ordinals by first use in canonical order.
#[test]
fn two_steps_then_merge_equal_one_step_byte_for_byte() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut f = Fixture::init(&dir.path().join("repo")).at("2024-01-01T00:00:00Z");
    let mut tips = Vec::new();
    for i in 0..14 {
        // Paths and authors reused out of first-use order, so a merge that kept the step
        // numbering would differ from the one-step build.
        f = f
            .write(&format!("src/f{}.rs", (13 - i) % 5), &format!("edit {i}\n"))
            .write(&format!("docs/d{i}.md"), "x\n");
        let who = format!("Dev{} <d{}@x.io>", i % 3, i % 3);
        f.git(&["add", "-A"]);
        f.git(&[
            "commit",
            "-q",
            "--author",
            &who,
            "-m",
            &format!("fix(core): step {i}"),
        ]);
        tips.push(f.git(&["rev-parse", "HEAD"]));
    }
    let head = tips.last().expect("tip").clone();
    let source = Repo::discover(f.path()).expect("open");

    let one_dir = dir.path().join("one");
    let one = Index::open_dir(one_dir.clone()).expect("index");
    one.update(&source, &head, &|_| {}).expect("one step");
    let want = live_files(&one_dir);
    assert_eq!(segments(&one_dir), 1);

    // First at HEAD~8, then one commit per update: each adds a segment, and the last makes
    // nine, more than the eight that start the merge.
    let two_dir = dir.path().join("two");
    let two = Index::open_dir(two_dir.clone()).expect("index");
    for tip in &tips[5..] {
        two.update(&source, tip, &|_| {}).expect("step");
        // Each step names a chain, and every file it names is on disk, not swept. The last
        // step starts the merge, which may swap the set under the check, so it is left out.
        if tip != &head {
            let has = kinds(&two_dir).contains(&(Kind::Chain as u8));
            assert!(has, "chain after a step");
        }
    }
    // The merge runs on a background thread: wait until the set is one segment again.
    gitterrace_testkit::wait::until_blocking("the merge ran", || segments(&two_dir) == 1);
    assert_eq!(
        kinds(&two_dir),
        kinds(&one_dir),
        "live kinds after the merge"
    );
    let got = live_files(&two_dir);
    assert_eq!(got.len(), want.len(), "live file count");
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g.0, w.0, "kind of live file {i}");
        assert!(
            g.1 == w.1,
            "live file {i} (kind {}) differs from the one-step build",
            w.0
        );
    }
    // The commit section is part of what was compared, not an empty one.
    let store = Store::open(&two_dir).expect("open").expect("set");
    let commits: u32 = store.segments[0]
        .blocks(section::COMMITS)
        .iter()
        .map(|e| e.items)
        .sum();
    assert_eq!(commits as usize, tips.len(), "commit section items");
}

/// The rename chain (a fork of one life included) built by the same appends with one worker,
/// four and ten, the ten twice: every live file byte for byte the same, section 7 included.
#[test]
fn an_append_sequence_with_a_fork_indexes_the_same_on_any_thread_count() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = gitterrace_testkit::RenameChain::build(dir.path());
    let stops = gitterrace_testkit::git(
        &r.repo,
        &["rev-list", "--first-parent", "--reverse", "HEAD"],
    );
    let stops: Vec<&str> = stops.lines().step_by(4).chain(["HEAD"]).collect();
    let build = |threads: usize, n: usize| {
        let source = Repo::discover(&r.repo).expect("open").with_workers(threads);
        let cache = dir.path().join(format!("c{threads}-{n}"));
        let index = Index::open_dir(cache.clone()).expect("index");
        for s in &stops {
            index.update(&source, s, &|_| {}).expect("append");
        }
        assert!(segments(&cache) > 1, "built by appends");
        let store = Store::open(&cache).expect("open").expect("set");
        let canon: u32 = store
            .segments
            .iter()
            .flat_map(|s| s.blocks(section::CANON))
            .map(|e| e.items)
            .sum();
        assert!(canon > 0, "section 7 is written");
        live_files(&cache)
    };
    let one = build(1, 0);
    for (threads, n) in [(4, 0), (10, 0), (10, 1)] {
        assert!(build(threads, n) == one, "{threads} threads, run {n}");
    }
}
