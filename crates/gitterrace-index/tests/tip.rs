//! Opening the insights of the commit the segment set was built for does not walk history.
//!
//! The tip marker names that commit. Without it every page open walked the whole history to
//! prove the set holds exactly the shown commit, seconds on a large repository.

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitterrace_git::Repo;
use gitterrace_index::{CommitRecord, CommitSource, Index};
use gitterrace_testkit::git;

/// The real repository, counting history walks.
struct Counting {
    repo: Repo,
    walks: AtomicUsize,
}

impl CommitSource for Counting {
    fn rev_list(&self, rev: &str) -> io::Result<Vec<String>> {
        self.walks.fetch_add(1, Ordering::Relaxed);
        CommitSource::rev_list(&self.repo, rev)
    }

    fn resolve(&self, rev: &str) -> Option<String> {
        CommitSource::resolve(&self.repo, rev)
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        CommitSource::record(&self.repo, id)
    }

    fn records(&self, ids: &[String]) -> io::Result<Vec<CommitRecord>> {
        CommitSource::records(&self.repo, ids)
    }

    fn map_identities(
        &self,
        rev: &str,
        pairs: &[(String, String)],
    ) -> io::Result<Vec<(String, String)>> {
        CommitSource::map_identities(&self.repo, rev, pairs)
    }
}

#[test]
fn the_built_tip_opens_without_a_history_walk() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for n in 0..3 {
        std::fs::write(path.join("a.txt"), format!("{n}\n")).expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", &format!("c{n}")]);
    }
    let src = Counting {
        repo: Repo::discover(path).expect("repo"),
        walks: AtomicUsize::new(0),
    };
    let index = Index::open_dir(dir.path().join("cache")).expect("index");
    index.update(&src, "HEAD", &|_| {}).expect("update");
    let tip = src.resolve("HEAD").expect("tip");
    src.walks.store(0, Ordering::Relaxed);
    let set = index.disk_source(&src, &tip).expect("open");
    assert!(set.is_some(), "the built tip reads the segment set");
    assert_eq!(
        src.walks.load(Ordering::Relaxed),
        0,
        "history walks on open"
    );
    // Another commit has no marker: the set is checked against its history.
    let first = git(path, &["rev-list", "--max-parents=0", "HEAD"]);
    let other = index.disk_source(&src, first.trim()).expect("open");
    assert!(other.is_none(), "an older commit is not the built set");
    assert!(src.walks.load(Ordering::Relaxed) > 0);
}
