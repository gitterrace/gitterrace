use gitterrace_testkit::Fixture;
use gitterrace_types::AnchorStatus;

use super::{Anchor, AnchorQuery, anchors_now};
use crate::Repo;

const BODY: &str = "one\ntwo\nthree\nfour\nfive\n";

fn repo() -> (tempfile::TempDir, Repo, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let f = Fixture::init(dir.path()).write("a.txt", BODY).write("b.txt", "b\n").commit("init");
    let sha = f.git(&["rev-parse", "HEAD"]);
    let repo = Repo::discover(dir.path()).expect("repo");
    (dir, repo, sha)
}

fn one(repo: &Repo, sha: &str, path: &str, lines: Option<(u32, u32)>) -> Anchor {
    let q = AnchorQuery { sha: sha.to_owned(), path: Some(path.to_owned()), lines };
    anchors_now(repo, &[q]).expect("anchors").remove(0)
}

fn commit(dir: &std::path::Path, msg: &str) {
    gitterrace_testkit::git(dir, &["add", "-A"]);
    gitterrace_testkit::git(dir, &["commit", "-q", "-m", msg]);
}

#[test]
fn unchanged_lines_are_untouched() {
    let (_d, repo, sha) = repo();
    let a = one(&repo, &sha, "a.txt", Some((2, 3)));
    let got = (a.status, a.now_path.as_deref(), a.now_lines);
    assert_eq!(got, (AnchorStatus::Untouched, Some("a.txt"), None));
    assert!(a.worktree);
}

#[test]
fn lines_that_moved_are_untouched_and_say_where() {
    let (d, repo, sha) = repo();
    std::fs::write(d.path().join("a.txt"), format!("new\nnew2\n{BODY}")).expect("write");
    commit(d.path(), "prepend");
    let a = one(&repo, &sha, "a.txt", Some((2, 3)));
    assert_eq!((a.status, a.now_lines), (AnchorStatus::Untouched, Some((4, 5))));
}

#[test]
fn edited_lines_say_lines_edited_and_a_range_past_the_end_says_file_changed() {
    let (d, repo, sha) = repo();
    std::fs::write(d.path().join("a.txt"), "one\nTWO\nthree\nfour\nfive\n").expect("write");
    commit(d.path(), "edit");
    assert_eq!(one(&repo, &sha, "a.txt", Some((2, 2))).status, AnchorStatus::LinesEdited);
    assert_eq!(one(&repo, &sha, "a.txt", Some((4, 5))).status, AnchorStatus::Untouched);
    assert_eq!(one(&repo, &sha, "a.txt", Some((4, 40))).status, AnchorStatus::FileChanged);
    assert_eq!(one(&repo, &sha, "a.txt", None).status, AnchorStatus::FileChanged);
    assert_eq!(one(&repo, &sha, "b.txt", None).status, AnchorStatus::Untouched);
}

#[test]
fn an_uncommitted_edit_counts_and_is_checked_against_the_worktree() {
    let (d, repo, sha) = repo();
    std::fs::write(d.path().join("a.txt"), "one\nTWO\nthree\nfour\nfive\n").expect("write");
    let a = one(&repo, &sha, "a.txt", Some((2, 2)));
    assert_eq!(a.status, AnchorStatus::LinesEdited);
    assert!(a.worktree);
}

#[test]
fn a_deleted_file_is_gone_and_a_renamed_one_keeps_its_content_under_the_new_name() {
    let (d, repo, sha) = repo();
    std::fs::remove_file(d.path().join("b.txt")).expect("rm");
    assert_eq!(one(&repo, &sha, "b.txt", None).status, AnchorStatus::FileGone);
    std::fs::rename(d.path().join("a.txt"), d.path().join("moved.txt")).expect("mv");
    let a = one(&repo, &sha, "a.txt", Some((2, 3)));
    assert_eq!((a.status, a.now_path.as_deref()), (AnchorStatus::Untouched, Some("moved.txt")));
    commit(d.path(), "move");
    let a = one(&repo, &sha, "a.txt", Some((2, 3)));
    assert_eq!((a.status, a.now_path.as_deref()), (AnchorStatus::Untouched, Some("moved.txt")));
    assert_eq!(one(&repo, &sha, "nope.txt", None).status, AnchorStatus::FileGone);
}

#[test]
fn a_selection_without_a_file_is_untouched() {
    let (_d, repo, sha) = repo();
    let q = AnchorQuery { sha, path: None, lines: None };
    assert_eq!(anchors_now(&repo, &[q]).expect("anchors")[0].status, AnchorStatus::Untouched);
}

fn edit(d: &std::path::Path, body: &str) {
    std::fs::write(d.join("a.txt"), body).expect("write");
    commit(d, "edit");
}

#[test]
fn an_edit_inside_the_range_says_lines_edited_at_the_mapped_span() {
    let (d, repo, sha) = repo();
    edit(d.path(), "top\none\ntwo\nTHREE\nthree b\nfour\nfive\n");
    let a = one(&repo, &sha, "a.txt", Some((2, 4)));
    assert_eq!((a.status, a.now_lines), (AnchorStatus::LinesEdited, Some((3, 6))));
}

#[test]
fn a_one_to_one_replacement_is_lines_edited_in_place() {
    let (d, repo, sha) = repo();
    edit(d.path(), "one\nTWO\nthree\nfour\nfive\n");
    let a = one(&repo, &sha, "a.txt", Some((2, 2)));
    assert_eq!((a.status, a.now_lines), (AnchorStatus::LinesEdited, Some((2, 2))));
}

#[test]
fn a_deleted_range_stays_lines_changed_with_no_place() {
    let (d, repo, sha) = repo();
    edit(d.path(), "one\nfour\nfive\n");
    let a = one(&repo, &sha, "a.txt", Some((2, 3)));
    assert_eq!((a.status, a.now_lines), (AnchorStatus::LinesChanged, None));
}

#[test]
fn an_uncommitted_edit_is_followed_through_the_worktree() {
    let (d, repo, sha) = repo();
    std::fs::write(d.path().join("a.txt"), "new\none\nTWO\nthree\nfour\nfive\n").expect("write");
    let a = one(&repo, &sha, "a.txt", Some((2, 3)));
    assert_eq!((a.status, a.now_lines), (AnchorStatus::LinesEdited, Some((3, 4))));
    assert!(a.worktree);
}

#[test]
fn an_unchanged_file_is_never_lines_edited() {
    let (_d, repo, sha) = repo();
    for range in [(1, 1), (1, 5), (3, 4)] {
        let a = one(&repo, &sha, "a.txt", Some(range));
        assert_eq!(a.status, AnchorStatus::Untouched, "{range:?}");
    }
}

#[test]
fn a_rename_plus_edit_follows_the_new_name() {
    let (d, repo, sha) = repo();
    std::fs::remove_file(d.path().join("a.txt")).expect("rm");
    std::fs::write(d.path().join("moved.txt"), "one\nTWO\nthree\nfour\nfive\n").expect("write");
    commit(d.path(), "move and edit");
    let a = one(&repo, &sha, "a.txt", Some((2, 2)));
    assert_eq!(a.status, AnchorStatus::LinesEdited);
    assert_eq!(a.now_path.as_deref(), Some("moved.txt"));
}

#[test]
fn the_diff_map_shifts_kept_lines_and_spans_replacements() {
    use super::{Run, Spot, map_range, spot};
    // Old line 1 (0-based) became two lines, and one line was added before line 3.
    let runs: Vec<Run> = vec![((1, 2), (1, 3)), ((3, 3), (4, 5))];
    assert_eq!(spot(&runs, 0), Spot::Kept(0));
    assert_eq!(spot(&runs, 1), Spot::Replaced(1, 3));
    assert_eq!(spot(&runs, 2), Spot::Kept(3));
    assert_eq!(spot(&runs, 3), Spot::Kept(5));
    assert_eq!(map_range(&runs, 0, 2), Some((1, 4)));
    assert_eq!(map_range(&[((1, 3), (1, 1))], 1, 2), None);
}
