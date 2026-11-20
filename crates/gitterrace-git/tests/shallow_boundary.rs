//! A `--depth` clone lists parents it does not hold; walks treat that commit as a root.

use std::time::Duration;

use gitterrace_git::{Repo, last_commits};
use gitterrace_testkit::Adversarial;

#[test]
fn last_commits_and_path_log_succeed_at_a_shallow_boundary() {
    let tmp = tempfile::tempdir().expect("tmp");
    let fx = Adversarial::build(tmp.path());
    let repo = Repo::discover(&fx.shallow).expect("open");
    let (found, _) =
        last_commits(&repo, "HEAD", "", Duration::from_secs(60)).expect("last commits");
    assert!(!found.is_empty());
    let log = repo.log("HEAD", Some(&found[0].name), 0, 100).expect("path log");
    assert!(!log.is_empty());
}
