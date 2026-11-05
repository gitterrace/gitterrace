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
    assert_eq!(files, ["c.rs", "d.rs", "foo.rs", "keep.rs", "r.rs"]);
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
