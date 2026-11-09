//! The index's merge rename limit: which renames a large merge pairs, and that every other
//! commit reads exactly as pages read it.

use gitterrace_testkit::git_at;
use std::fs;

use crate::Repo;
use crate::diff::{MERGE_RENAME_LIMIT, Renames};
use crate::repo::tests::{T0, commit, numbered, write};

/// A side branch that moves `n` files into `moved/`, the first `edited` of them also edited,
/// and is merged back, so the merge's first-parent diff adds and deletes `n` files each. Then
/// non-merge commits with edited and identical renames. gix applies the limit to the pairs
/// left after identical ones are matched, so the edited moves are what crosses it.
fn merged_moves(d: &std::path::Path, n: usize, edited: usize) {
    git_at(d, T0, &["init", "-q", "-b", "main"]);
    for i in 0..n {
        write(d, &format!("a/{i:03}.txt"), &numbered(20 + i));
    }
    commit(d, "root");
    git_at(d, T0, &["checkout", "-q", "-b", "side"]);
    fs::create_dir(d.join("moved")).expect("mkdir");
    for i in 0..n {
        fs::rename(
            d.join(format!("a/{i:03}.txt")),
            d.join(format!("moved/{i:03}.txt")),
        )
        .expect("move");
    }
    for i in 0..edited {
        write(
            d,
            &format!("moved/{i:03}.txt"),
            &format!("{}edited\n", numbered(20 + i)),
        );
    }
    commit(d, "move everything");
    git_at(d, T0, &["checkout", "-q", "main"]);
    write(d, "other.txt", "x\n");
    commit(d, "main moves on");
    git_at(
        d,
        T0,
        &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
    );
    fs::rename(d.join("other.txt"), d.join("other2.txt")).expect("rename");
    write(d, "moved/001.txt", &format!("{}more\n", numbered(21)));
    fs::rename(d.join("moved/001.txt"), d.join("moved/one.txt")).expect("rename");
    commit(d, "plain renames");
}

/// Changes of `id` as (kind, path, old path), sorted.
fn kinds(repo: &Repo, id: &str, renames: Renames) -> Vec<String> {
    let gix = repo.repo();
    let commit = gix
        .find_commit(repo.full_id(&gix, id).expect("id"))
        .expect("commit");
    let mut cache = crate::diff::resource_cache(&gix).expect("cache");
    let files = crate::diff::changes_with(
        &gix,
        &commit,
        false,
        crate::MAX_TEXT_BYTES,
        &mut cache,
        None,
        renames,
    )
    .expect("diff");
    let mut out: Vec<String> = files
        .iter()
        .map(|f| format!("{:?} {} {:?}", f.kind, f.path, f.old_path))
        .collect();
    out.sort();
    out
}

/// Guard class: the index's rename rule reaching beyond large merges. Every non-merge commit
/// (renames edited and identical included) diffs byte for byte as pages diff it.
#[test]
fn non_merge_commits_diff_the_same_for_the_index_and_for_pages() {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    merged_moves(d, MERGE_RENAME_LIMIT + 2, MERGE_RENAME_LIMIT + 1);
    let repo = Repo::discover(d).expect("repo");
    let gix = repo.repo();
    let mut checked = 0;
    for id in repo.rev_list("HEAD").expect("rev_list") {
        let commit = gix
            .find_commit(repo.full_id(&gix, &id).expect("id"))
            .expect("commit");
        if commit.parent_ids().nth(1).is_some() {
            continue;
        }
        let mut cache = crate::diff::resource_cache(&gix).expect("cache");
        let mut run = |r| {
            let files = crate::diff::changes_with(
                &gix,
                &commit,
                false,
                crate::MAX_TEXT_BYTES,
                &mut cache,
                None,
                r,
            )
            .expect("diff");
            format!("{files:?}")
        };
        assert_eq!(run(Renames::Index), run(Renames::Full), "commit {id}");
        checked += 1;
    }
    assert_eq!(checked, 4);
}

/// A merge past the limit pairs only renames of identical content: each edited move is a
/// delete plus an add for the index, while pages still pair it. A merge under the limit pairs
/// the edited moves too.
#[test]
fn a_large_merge_pairs_only_identical_renames_for_the_index() {
    let over = MERGE_RENAME_LIMIT + 1;
    for (n, edited, paired_edit) in [(over + 2, over, false), (4_usize, 2_usize, true)] {
        let tmp = tempfile::tempdir().expect("tmp");
        let d = tmp.path();
        merged_moves(d, n, edited);
        let repo = Repo::discover(d).expect("repo");
        let merge = git_at(d, T0, &["rev-parse", "HEAD^"]);
        let merge = merge.trim();
        let index = kinds(&repo, merge, Renames::Index);
        let pages = kinds(&repo, merge, Renames::Full);
        let first = "Renamed moved/000.txt Some(\"a/000.txt\")".to_owned();
        assert!(
            pages.contains(&first),
            "pages pair the edited move: {pages:?}"
        );
        assert_eq!(index.contains(&first), paired_edit, "n {n}: {index:?}");
        let renamed = |v: &[String]| v.iter().filter(|k| k.starts_with("Renamed")).count();
        assert_eq!(renamed(&pages), n);
        assert_eq!(renamed(&index), if paired_edit { n } else { n - edited });
        if !paired_edit {
            assert!(index.contains(&"Added moved/000.txt None".to_owned()));
            assert!(index.contains(&"Deleted a/000.txt None".to_owned()));
        }
    }
}
