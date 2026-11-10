//! The head tree is read once per commit: later insight requests walk no tree.

use gitterrace_testkit::git;

use crate::{ApiState, Params};

fn state(dir: &std::path::Path) -> ApiState {
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join(".gitignore"), "skip.txt\n").expect("write");
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").expect("write");
    std::fs::write(dir.join("skip.txt"), "x\n").expect("write");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "first"]);
    let s = ApiState::open(dir).expect("open");
    s.refresh_index().expect("index");
    s
}

#[test]
fn insights_after_the_first_walk_no_tree() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    let params = Params {
        period: Some("all".to_owned()),
        ..Params::default()
    };
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
