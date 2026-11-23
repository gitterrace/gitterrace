use gitterrace_testkit::{Fixture, git};

use axum::http::StatusCode;

use crate::ApiState;
use crate::tests::{fixture, get};

/// A branch with one commit of its own, `main` moved on after the split.
fn forked() -> (tempfile::TempDir, ApiState) {
    let (d, _) = fixture();
    let p = d.path();
    git(p, &["checkout", "-q", "-b", "side", "HEAD~1"]);
    std::fs::write(p.join("side.txt"), "s\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: side"]);
    git(p, &["checkout", "-q", "main"]);
    let state = ApiState::open(p).expect("open");
    (d, state)
}

fn paths(c: &gitterrace_types::api::CompareResponse) -> Vec<&str> {
    c.files.iter().map(|f| f.path.as_str()).collect()
}

/// Two dots compare the two trees, three dots only what head did since the split.
#[test]
fn compare_three_dot_leaves_out_what_base_did_alone() {
    let (_d, s) = forked();
    let two = s.compare("main", Some("side"), None, Some(false), None).expect("two");
    let three = s.compare("main", Some("side"), None, Some(true), None).expect("three");
    assert!(paths(&two).contains(&"src/b.rs"), "{:?}", paths(&two));
    assert_eq!(paths(&three), ["side.txt"]);
    assert_ne!(two.from, three.from);
    assert!(three.files[0].hunks.is_some());
}

#[test]
fn compare_path_keeps_only_files_under_it() {
    let (_d, s) = forked();
    let c = s.compare("main", Some("side"), Some("src/"), Some(false), None).expect("compare");
    assert_eq!(paths(&c), ["src/b.rs"]);
    let none = s.compare("main", Some("side"), Some("sr"), Some(false), None).expect("compare");
    assert!(none.files.is_empty(), "a prefix is not a directory");
}

/// The caller cannot lift the diff cap, and hostile revs are refused like everywhere else.
#[test]
fn compare_stays_bounded_and_refuses_hostile_revs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let _ = Fixture::init(p).write("a.txt", "1\n").commit("one");
    let big = "x\n".repeat(gitterrace_config::Api::default().diff_lines + 10);
    std::fs::write(p.join("big.txt"), big).expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "big"]);
    let s = ApiState::open(p).expect("open");
    for asked in [None, Some(usize::MAX)] {
        let c = s.compare("HEAD~1", Some("HEAD"), None, Some(false), asked).expect("compare");
        let lines: usize =
            c.files.iter().flat_map(|f| f.hunks.iter().flatten()).map(|h| h.lines.len()).sum();
        assert!(lines <= gitterrace_config::Api::default().diff_lines, "{asked:?}");
        assert!(c.truncated, "{asked:?}");
        assert!(c.additions > lines as u64, "totals count the cut lines");
    }
    for rev in [":/one", "HEAD^{/one}"] {
        assert!(
            matches!(
                s.compare(rev, Some("HEAD"), None, Some(false), None),
                Err(crate::ApiError::BadRequest(_))
            ),
            "{rev}"
        );
        assert!(
            matches!(
                s.compare("HEAD", Some(rev), None, Some(false), None),
                Err(crate::ApiError::BadRequest(_))
            ),
            "{rev}"
        );
    }
    assert!(s.compare("nope", Some("HEAD"), None, Some(false), None).is_err());
}

#[test]
fn worktree_diff_lists_uncommitted_files_and_refuses_unknown_scopes() {
    let (d, s) = fixture();
    std::fs::write(d.path().join("a.txt"), "changed\n").expect("write");
    let w = s.worktree_diff(Some("all"), None).expect("worktree");
    assert_eq!(w.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    for bad in ["", "everything", "ALL"] {
        assert!(
            matches!(s.worktree_diff(Some(bad), None), Err(crate::ApiError::BadRequest(_))),
            "{bad:?}"
        );
    }
}

/// Each scope reads its own pair of trees: a staged-only file and an unstaged-only file land
/// in their own scope and both in all, so a scope that read the wrong pair lists the wrong
/// files.
#[test]
fn worktree_diff_scopes_list_staged_unstaged_and_all_files() {
    let (d, s) = fixture();
    let p = d.path();
    std::fs::write(p.join("a.txt"), "staged\n").expect("write");
    git(p, &["add", "a.txt"]);
    std::fs::write(p.join("src/b.rs"), "fn unstaged() {}\n").expect("write");
    let paths = |scope: &str| {
        let w = s.worktree_diff(Some(scope), None).expect("worktree");
        assert_eq!(w.scope, scope);
        w.files.into_iter().map(|f| f.path).collect::<Vec<_>>()
    };
    assert_eq!(paths("staged"), ["a.txt"]);
    assert_eq!(paths("unstaged"), ["src/b.rs"]);
    assert_eq!(paths("all"), ["a.txt", "src/b.rs"]);
}

/// The working tree is this machine's, not the repository's: a public handle never reads it,
/// on either transport.
#[tokio::test]
async fn worktree_diff_is_refused_on_a_public_server() {
    let (d, s) = fixture();
    std::fs::write(d.path().join("a.txt"), "secret\n").expect("write");
    let s = s.committed_only();
    assert!(matches!(s.worktree_diff(Some("all"), None), Err(crate::ApiError::Forbidden(_))));
    let (st, v) = get(&s, "/worktree").await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert!(!v.to_string().contains("secret"));
    let (st, v) = get(&s, "/compare?base=HEAD~1").await;
    assert_eq!(st, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn compare_endpoint_needs_a_base() {
    let (_d, s) = fixture();
    let (st, v) = get(&s, "/compare").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v) = get(&s, "/compare?base=HEAD~1&three_dot=true").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["three_dot"], true);
}

/// A checkpoint then an edit: `since` lists only the edited file, counts the rest and answers
/// the same JSON bytes as before when `since` is not given.
#[test]
fn worktree_since_a_checkpoint_lists_only_what_changed_after_it() {
    let (d, s) = fixture();
    let p = d.path();
    std::fs::write(p.join("a.txt"), "edited\n").expect("write");
    std::fs::write(p.join("src/b.rs"), "fn edited() {}\n").expect("write");
    let (id, new) = s.checkpoint_create("", false).expect("checkpoint");
    assert!(new && id.starts_with("cp-"));
    assert_eq!(s.checkpoint_create("", false).expect("again"), (id.clone(), false));
    std::fs::write(p.join("a.txt"), "edited twice\n").expect("write");
    let w = s.worktree_diff_since(Some("all"), None, Some(&id)).expect("since");
    assert_eq!(w.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    assert_eq!((w.since.as_deref(), w.unchanged_files), (Some(id.as_str()), Some(1)));
    assert_eq!(w.content_gone.len(), 0);
    let plain =
        serde_json::to_value(s.worktree_diff(Some("all"), None).expect("plain")).expect("json");
    for k in ["since", "unchanged_files", "content_gone"] {
        assert!(plain.get(k).is_none(), "{k} must stay out of an answer without since");
    }
    assert!(matches!(
        s.worktree_diff_since(Some("all"), None, Some("cp-000000000000")),
        Err(crate::ApiError::CheckpointNotFound(_))
    ));
    assert!(matches!(
        s.worktree_diff_since(Some("staged"), None, Some(&id)),
        Err(crate::ApiError::BadRequest(_))
    ));
    let list = s.checkpoints().expect("list");
    assert!(list.persisted && list.checkpoints.len() == 1 && list.cap_bytes == 20 << 20);
    assert_eq!(list.checkpoints[0].files, 2);
}

#[test]
fn viewed_marks_round_trip_through_the_state() {
    let (_d, s) = fixture();
    s.viewed_set(&[("a.txt".to_owned(), "9f2c41ab77d0e1c3".to_owned())], true).expect("set");
    let v = s.viewed().expect("viewed").viewed;
    assert_eq!((v[0].path.as_str(), v[0].patch.as_str()), ("a.txt", "9f2c41ab77d0e1c3"));
    s.viewed_set(&[("a.txt".to_owned(), String::new())], false).expect("clear");
    assert_eq!(s.viewed().expect("viewed").viewed.len(), 0);
}

/// Checkpoints and marks are this machine's: a public handle refuses the reads and the writes,
/// and the routes answer 403.
#[tokio::test]
async fn checkpoints_and_viewed_are_refused_on_a_public_server() {
    let (_d, s) = fixture();
    let s = s.committed_only();
    assert!(matches!(s.checkpoints(), Err(crate::ApiError::Forbidden(_))));
    assert!(matches!(s.viewed(), Err(crate::ApiError::Forbidden(_))));
    assert!(matches!(s.checkpoint_create("", false), Err(crate::ApiError::Forbidden(_))));
    assert!(matches!(s.viewed_set(&[], true), Err(crate::ApiError::Forbidden(_))));
    for uri in ["/checkpoints", "/viewed", "/worktree?since=cp-1"] {
        let (st, v) = get(&s, uri).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{uri}: {v}");
    }
}

#[test]
fn checkpoints_are_off_when_the_cap_is_zero() {
    let (d, s) = fixture();
    std::fs::write(d.path().join("a.txt"), "x\n").expect("write");
    let cfg = gitterrace_config::parse("[review]\ncheckpoint_mb = 0\n").expect("parse").config;
    let s = s.configured(&cfg);
    let l = s.checkpoints().expect("list");
    assert!(!l.persisted && l.checkpoints.is_empty() && l.cap_bytes == 0);
    assert!(matches!(s.checkpoint_create("", false), Err(crate::ApiError::BadRequest(_))));
}
