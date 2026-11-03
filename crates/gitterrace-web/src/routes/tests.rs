//! Tests for the route handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use gitterrace_index::{CommitRecord, FileStat};

use crate::routes::{commit, indexed_author_ids};

/// Guard: after the index is ready, the costliest files are blamed off the request path, so
/// the first visitor reads the cache. Ranking is size times churn, and it stops at N.
#[tokio::test]
async fn blame_prewarm_fills_the_cache_for_the_costliest_files() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = gitterrace_testkit::Fixture::init(dir.path())
        .write("big.txt", "a\nb\nc\nd\n")
        .write("once.txt", "x\n")
        .write("small.txt", "1\n")
        .commit("feat: a")
        .write("big.txt", "a\nB\nc\nd\n")
        .write("small.txt", "2\n")
        .commit("fix: b");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = Arc::new(crate::app::App::configured(repo, index, &env));
    app.index
        .update(&app.repo, "HEAD", &|_| {})
        .expect("update");
    app.mark_ready();
    let head = app.repo.resolve("HEAD").expect("head");
    let src = app.source(&head).expect("source").expect("ready");
    let files = app.head_lines(&head);
    assert_eq!(
        super::files::blame_warm_list(&files, &*src, 5),
        ["big.txt", "small.txt"],
        "never rewritten is cheap and skipped, the rest ranked by cost"
    );
    assert_eq!(super::files::blame_warm_list(&files, &*src, 1), ["big.txt"]);
    assert!(!app.repo.blame_is_cached(&head, "big.txt"));
    super::prewarm_blame(Arc::clone(&app)).await;
    assert!(app.repo.blame_is_cached(&head, "big.txt"));
    assert!(app.repo.blame_is_cached(&head, "small.txt"));
    assert!(!app.repo.blame_is_cached(&head, "once.txt"));
}

/// Guard: the commit page (diff and diffstat) is rendered once per commit. The memo entry
/// is swapped for a marker; a handler that diffs again would not return it.
#[tokio::test]
async fn commit_page_is_memoized_per_commit() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = gitterrace_testkit::Fixture::init(dir.path())
        .write("a.txt", "one\n")
        .commit("feat: a")
        .write("a.txt", "two\n")
        .commit("fix: b");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let head = repo.resolve("HEAD").expect("head");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = Arc::new(crate::app::App::configured(repo, index, &env));
    let first = commit(State(Arc::clone(&app)), Path("HEAD".into()))
        .await
        .expect("page")
        .0;
    assert!(first.contains("fix: b"), "{first}");
    app.memo_put(app.memo_key("commit", Some(&head)), Arc::new("memo".into()));
    let again = commit(State(Arc::clone(&app)), Path(head.clone()))
        .await
        .expect("page")
        .0;
    assert_eq!(again, "memo", "the repeat must not diff again");
    let parent = commit(State(app), Path("HEAD~1".into()))
        .await
        .expect("page")
        .0;
    assert!(
        parent.contains("feat: a"),
        "another commit renders its own page"
    );
}

fn rec(id: &str, author: &str, email: &str, path: &str) -> CommitRecord {
    CommitRecord {
        id: id.into(),
        author: author.into(),
        email: email.into(),
        time: 0,
        offset: 0,
        merge: false,
        parents: Vec::new(),
        kind: None,
        scope: None,
        files: vec![FileStat {
            path: path.into(),
            ..FileStat::default()
        }],
    }
}

#[test]
fn author_log_comes_from_the_records_newest_first() {
    let list = [
        rec("a", "Ann", "ann@x.y", "src/a.rs"),
        rec("b", "Bob", "bob@x.y", "src/b.rs"),
        rec("c", "Ann", "ANN@x.y", "docs/c.md"),
    ];
    let recs = gitterrace_index::feed::MemorySource::new(&list);
    assert_eq!(indexed_author_ids(&recs, "ann@x.y", None), ["c", "a"]);
    assert_eq!(indexed_author_ids(&recs, "Ann", Some("src")), ["a"]);
    assert!(indexed_author_ids(&recs, "nobody", None).is_empty());
}
