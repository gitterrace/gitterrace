//! The head tree is read once per commit: later insight requests walk no tree.

use gitterrace_testkit::Fixture;

use crate::{ApiState, Params};

fn state(dir: &std::path::Path) -> ApiState {
    let _ = Fixture::init(dir)
        .write(".gitignore", "skip.txt\n")
        .write("a.txt", "one\ntwo\n")
        .write("skip.txt", "x\n")
        .commit("first");
    let s = ApiState::open(dir).expect("open");
    s.refresh_index().expect("index");
    s
}

#[test]
fn insights_after_the_first_walk_no_tree() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    let params = Params { period: Some("all".to_owned()), ..Params::default() };
    let first = s.insight("hotspots", None, &params).expect("first");
    let walked = s.repo.tree_walks();
    assert!(walked > 0, "the first request reads the tree");
    for name in ["hotspots", "ownership", "contributors"] {
        s.insight(name, None, &params).expect(name);
    }
    assert_eq!(s.repo.tree_walks(), walked, "later requests walk no tree");
    let again = s.insight("hotspots", None, &params).expect("again");
    assert_eq!(again.data, first.data);
}

#[test]
fn release_records_empties_the_head_memo() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    let params = Params { period: Some("all".to_owned()), ..Params::default() };
    // Hotspots read both the ignore files and the head line counts.
    s.insight("hotspots", None, &params).expect("hotspots");
    assert!(s.head_memo_held(), "a request fills the head memo");
    s.release_records();
    assert!(!s.head_memo_held(), "the release keeps a per-commit value");
}

#[test]
fn insight_requests_over_one_tip_build_the_derived_tables_once() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    let params = Params { period: Some("all".to_owned()), ..Params::default() };
    s.insight("summary", None, &params).expect("first");
    let src = s.records("HEAD").expect("records");
    let built = src.share().run(|a| gitterrace_index::table_builds(&**a)).expect("run");
    assert_eq!(built, 1, "the first request builds the tables");
    for name in ["summary", "contributors", "hotspots"] {
        s.insight(name, None, &params).expect(name);
    }
    let after = src.run(|a| gitterrace_index::table_builds(&**a)).expect("run");
    assert_eq!(after, 1, "later requests over the same tip slice the same tables");
}
