//! Rename-aware aggregates against git itself, on the rename chain fixture.
//!
//! The oracle is `git log --follow --no-merges` per file at the tip, cut at the newest
//! deletion of the name it follows: a re-added file starts a new life, so the rows before its
//! deletion are not its own. A copy starts a life: git `--follow` would go on into the
//! source's history, the counts do not.

use std::collections::BTreeMap;

use gitterrace_git::Repo;
use gitterrace_index::Index;
use gitterrace_index::feed::InsightsSource;
use gitterrace_testkit::{MergeWithAuthor, RenameChain, git};

/// Per file at the tip, the non-merge commits of its life, newest first.
pub fn follow_oracle(repo: &std::path::Path) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for file in git(repo, &["ls-files"]).lines() {
        let log = git(
            repo,
            &[
                "log",
                "--follow",
                "--no-merges",
                "--name-status",
                "--format=%H",
                "--",
                file,
            ],
        );
        let mut life = Vec::new();
        let mut commit = String::new();
        for line in log.lines().filter(|l| !l.is_empty()) {
            if !line.contains('\t') {
                line.clone_into(&mut commit);
            } else if line.starts_with('D') {
                break;
            } else if line.starts_with('C') {
                // The copy starts this file's life: git would go on into the source's.
                life.push(commit.clone());
                break;
            } else {
                life.push(commit.clone());
            }
        }
        out.insert(file.to_owned(), life);
    }
    out
}

/// Builds the cache of `repo` and opens what pages read.
pub fn built(repo: &std::path::Path) -> std::sync::Arc<gitterrace_index::DiskSet> {
    let r = Repo::discover(repo).expect("repo");
    let index = Index::open(&r).expect("index");
    index.update(&r, "HEAD", &|_| {}).expect("build");
    let tip = git(repo, &["rev-parse", "HEAD"]);
    index
        .disk_source(&r, &tip)
        .expect("open")
        .expect("the built tip has a set")
        .unchecked()
}

#[test]
fn the_rename_chain_has_every_case() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = RenameChain::build(dir.path());
    let files = git(&r.repo, &["ls-files"]);
    let files: Vec<&str> = files.lines().collect();
    assert_eq!(
        files,
        [
            "c.rs", "d.rs", "foo.rs", "keep.rs", "r.rs", "y2.rs", "z2.rs"
        ]
    );
    let renames = git(&r.repo, &["log", "--format=", "--name-status", "-M"]);
    for pair in ["a.rs\tb.rs", "b.rs\tc.rs", "x.rs\ty.rs", "Foo.rs\tfoo.rs"] {
        assert!(renames.contains(pair), "rename {pair}: {renames}");
    }
    for gone in ["D\ty.rs", "D\tr.rs", "A\td.rs"] {
        assert!(renames.contains(gone), "{gone}: {renames}");
    }
    let when = git(&r.repo, &["show", "-s", "--format=%ai", &r.month_end]);
    assert_eq!(when, "2024-01-31 23:30:00 -0800");
    let oracle = follow_oracle(&r.repo);
    let len = |f: &str| oracle[f].len();
    assert_eq!(
        (len("c.rs"), len("d.rs"), len("foo.rs"), len("r.rs")),
        (8, 1, 3, 2)
    );
    // The fork: the add of x2, the history before it, goes to y2 only, in git too.
    let add = &oracle["y2.rs"][2];
    assert!(!oracle["z2.rs"].contains(add), "{oracle:?}");
}

#[test]
fn the_merge_author_has_no_other_commit_that_day() {
    let dir = tempfile::tempdir().expect("tmp");
    let m = MergeWithAuthor::build(dir.path());
    let who = git(
        &m.repo,
        &[
            "show",
            "-s",
            "--format=%an %ad %p",
            "--date=short",
            &m.merge,
        ],
    );
    let parts: Vec<&str> = who.split(' ').collect();
    assert_eq!(parts.len(), 4, "a two-parent merge: {who}");
    assert_eq!(&parts[..2], ["Mia", "2024-03-04"]);
    let mia = git(
        &m.repo,
        &["log", "--no-merges", "--author=Mia", "--format=%H"],
    );
    assert!(mia.is_empty(), "Mia has a non-merge commit: {mia}");
}

/// The path x author aggregate a page reads, summed per file at the tip, counts the file's
/// whole life under every name it had: what `git log --follow` lists. Red while the
/// aggregate is keyed by the raw path (each name keeps only its own rows).
#[test]
#[ignore = "red until the path x author aggregate is keyed by the canonical life"]
fn canonical_touches_equal_git_log_follow() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = RenameChain::build(dir.path());
    let set = built(&r.repo);
    let mut touches: BTreeMap<String, u64> = BTreeMap::new();
    for row in set.agg_path_author("") {
        let name = set.path(row.path).expect("path");
        *touches.entry(name.to_string()).or_default() += row.touches;
    }
    let mut bad = Vec::new();
    for (file, life) in follow_oracle(&r.repo) {
        let got = touches.get(&file).copied().unwrap_or(0);
        if got != life.len() as u64 {
            bad.push(format!("{file}: {got} touches, git follow {}", life.len()));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Per tip file, the touches section 10 holds for the life its name holds.
fn life_touches(repo: &std::path::Path) -> BTreeMap<String, u64> {
    let set = built(repo);
    let store = gitterrace_index::seg::read::Store::open(&repo.join(".git/gitterrace"))
        .expect("store")
        .expect("a set");
    let (canon, touches) = gitterrace_index::seg::canon::dump(&store).expect("dump");
    let paths = set.paths();
    assert_eq!(canon.len() - paths, touches.len(), "CANON = paths + lives");
    let mut out = BTreeMap::new();
    for file in git(repo, &["ls-files"]).lines() {
        let id = set
            .find_path(file)
            .expect("path lookup reads")
            .expect("tip file has a path id")
            .0;
        let held = canon[id as usize];
        assert_ne!(held & (1 << 31), 0, "{file} holds a life");
        let life = (held & !(1 << 31)) as usize;
        assert_eq!(
            canon[paths + life],
            id | (1 << 31),
            "{file} names its live life"
        );
        out.insert(file.to_owned(), touches[life]);
    }
    out
}

/// Section 10 summed per life equals git log --follow for every tip file. When two branches
/// rename one file differently (y2 and z2), the history before the fork goes to the rename
/// first in the set's order and the other starts at its rename, the same rule as the owner's
/// decision that copies are not followed. Git agrees on this fixture. Where its date-ordered
/// walk gives the shared history to both names, the rule decides and git does not.
#[test]
fn canonical_lives_equal_git_log_follow() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = RenameChain::build(dir.path());
    let got = life_touches(&r.repo);
    let mut bad = Vec::new();
    for (file, life) in follow_oracle(&r.repo) {
        let n = got[&file];
        if n != life.len() as u64 {
            bad.push(format!("{file}: {n} touches, git follow {}", life.len()));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Commits of `repo` oldest first along the first parent, the points an append stops at.
fn first_parents(repo: &std::path::Path) -> Vec<String> {
    let log = git(repo, &["rev-list", "--first-parent", "--reverse", "HEAD"]);
    log.lines().map(str::to_owned).collect()
}

/// Builds the cache of `repo` by appends: an update at every `step`th first-parent commit,
/// then at HEAD. Returns the index dir.
pub fn appended(repo: &std::path::Path, cache: &std::path::Path, step: usize) {
    let r = Repo::discover(repo).expect("repo");
    let index = Index::open_dir(cache.to_owned()).expect("index");
    for id in first_parents(repo).iter().step_by(step.max(1)) {
        index.update(&r, id, &|_| {}).expect("append");
    }
    index.update(&r, "HEAD", &|_| {}).expect("append");
}

/// Section 7 and the life touches of a set built by appends equal a re-derivation from its
/// records in its own order file, the order the appends visited. Red when an append starts
/// its lives over instead of continuing the newest section 7.
#[test]
fn incremental_append_equals_full_rebuild() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = RenameChain::build(dir.path());
    for step in [3, 5] {
        let cache = dir.path().join(format!("appended-{step}"));
        appended(&r.repo, &cache, step);
        let store = gitterrace_index::seg::read::Store::open(&cache)
            .expect("store")
            .expect("a set");
        assert!(store.segments.len() > 1, "step {step} built by appends");
        let got = gitterrace_index::seg::canon::dump(&store).expect("dump");
        let src = Repo::discover(&r.repo).expect("repo");
        let index = Index::open_dir(cache.clone()).expect("index");
        let disk = index
            .disk_source(&src, &git(&r.repo, &["rev-parse", "HEAD"]))
            .expect("open")
            .expect("set")
            .unchecked();
        let records: Vec<_> = disk
            .base()
            .records()
            .collect::<Result<_, _>>()
            .expect("records");
        let want = gitterrace_index::seg::canon::canon_of(&records);
        assert_eq!(got, want, "step {step}");
    }
}

/// Section 7 of every segment is 4 B per path id the set knows plus 4 B per life.
#[test]
fn canon_bytes_are_four_per_path_and_four_per_life() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = RenameChain::build(dir.path());
    let set = built(&r.repo);
    let store = gitterrace_index::seg::read::Store::open(&r.repo.join(".git/gitterrace"))
        .expect("store")
        .expect("a set");
    let (canon, lives) = gitterrace_index::seg::canon::dump(&store).expect("dump");
    let bytes: u64 = store.segments[0]
        .blocks(gitterrace_index::seg::format::section::CANON)
        .iter()
        .map(|e| u64::from(e.len))
        .sum();
    assert_eq!(canon.len(), set.paths() + lives.len());
    assert_eq!(bytes, 4 * (set.paths() + lives.len()) as u64);
}
