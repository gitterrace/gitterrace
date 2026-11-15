//! Tests for the route handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use gitterrace_index::{CommitRecord, FileStat};

use crate::routes::commit;

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
    let src = app
        .source(&head)
        .expect("source")
        .expect("ready")
        .unchecked();
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

/// Guard class: background work an export pays for but never writes out. An export writes no
/// blame page, and one blame of a large, often edited file holds every past version of it (a
/// gigabyte on rust), so an export run (the one env with a change memo) warms no blame. Red
/// when the warm-up ignores what kind of run it serves.
#[tokio::test]
async fn an_export_run_warms_no_blame() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = gitterrace_testkit::Fixture::init(dir.path())
        .write("big.txt", "a\nb\n")
        .commit("feat: a")
        .write("big.txt", "a\nB\n")
        .commit("fix: b");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let mut env = crate::app::Env::new(gitterrace_config::Config::default());
    env.changes = Some(Arc::new(gitterrace_git::ChangeMemo::new()));
    let app = Arc::new(crate::app::App::configured(repo, index, &env));
    app.index
        .update(&app.repo, "HEAD", &|_| {})
        .expect("update");
    app.mark_ready();
    let head = app.repo.resolve("HEAD").expect("head");
    super::prewarm_blame(Arc::clone(&app)).await;
    assert!(!app.repo.blame_is_cached(&head, "big.txt"));
}

/// Guard class: a page body that grows with the file. A blame of many lines arrives as a page
/// of at most `BLAME_PAGE_LINES` rows plus pages fetched through `?from=`, and following the
/// pages yields every line once, in order. Red when any one answer holds the whole file (the
/// 13 MB rust blame), or a page skips or repeats a line.
#[tokio::test]
async fn a_long_blame_arrives_in_bounded_pages() {
    use axum::extract::Query;

    use super::files::{BLAME_PAGE_LINES, BlameQuery};

    let lines = 2 * BLAME_PAGE_LINES + 7;
    let body = "line\n".repeat(lines);
    let dir = tempfile::tempdir().expect("tmp");
    let f = gitterrace_testkit::Fixture::init(dir.path())
        .write("big.txt", &body)
        .commit("feat: a");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = Arc::new(crate::app::App::configured(repo, index, &env));
    let get = |from: Option<usize>| {
        let app = Arc::clone(&app);
        async move {
            super::blame(
                State(app),
                Path(("HEAD".into(), "big.txt".into())),
                Query(BlameQuery { from }),
            )
            .await
            .expect("blame")
            .0
        }
    };
    let mut seen = Vec::new();
    let mut page = get(None).await;
    loop {
        let rows = page.matches("<tr id=\"L").count();
        assert!(rows <= BLAME_PAGE_LINES, "{rows} rows in one answer");
        assert!(page.len() < BLAME_PAGE_LINES * 1024, "{} bytes", page.len());
        seen.extend(
            page.split("<tr id=\"L")
                .skip(1)
                .filter_map(|s| s.split('"').next()?.parse::<usize>().ok()),
        );
        let Some(next) = page.split("data-more=\"").nth(1) else {
            break;
        };
        let from = next
            .split("?from=")
            .nth(1)
            .and_then(|s| s.split('"').next()?.parse().ok())
            .expect("from");
        page = get(Some(from)).await;
    }
    assert_eq!(seen, (1..=lines).collect::<Vec<_>>());
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
    let ids = |q: &str| {
        let m = gitterrace_index::query::Matcher::new(
            &gitterrace_index::query::Query::parse(q).expect("query"),
        );
        let (ranks, _) = crate::routes::log_query::indexed(&recs, &m);
        ranks
            .iter()
            .rev()
            .filter_map(|r| {
                gitterrace_index::feed::InsightsSource::commit_id(&recs, *r).map(|s| s.to_string())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(ids("author:ann@x.y"), ["c", "a"]);
    assert_eq!(ids("author:Ann path:src"), ["a"]);
    assert!(ids("author:nobody").is_empty());
    assert_eq!(ids("author:Ann,bob@x.y !path:docs"), ["b", "a"]);
    assert_eq!(ids("!author:Ann"), ["b"]);
}

/// Guard class: a memoized page keyed on a rev is never served once the rev moves. The memo
/// entry is swapped for a marker: a repeat returns it, a new commit on the branch must not.
#[tokio::test]
async fn tree_page_is_memoized_until_the_branch_moves() {
    use axum::extract::Query;
    let dir = tempfile::tempdir().expect("tmp");
    let f = gitterrace_testkit::Fixture::init(dir.path())
        .write("a.txt", "one\n")
        .commit("feat: a");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = Arc::new(crate::app::App::configured(repo, index, &env));
    let get = |app: &Arc<crate::app::App>| {
        super::files::tree(
            State(Arc::clone(app)),
            Path(("main".into(), String::new())),
            Query(crate::routes::DocQuery { doc: None }),
        )
    };
    let first = get(&app).await.expect("page").0;
    assert!(first.contains("a.txt"), "{first}");
    let head = app.repo.resolve("main").expect("head");
    let key = app.memo_key("tree", Some(&format!("main\0{head}\0\0")));
    app.memo_put(key, Arc::new("memo".into()));
    assert_eq!(
        get(&app).await.expect("page").0,
        "memo",
        "a repeat is not rebuilt"
    );
    let _f = f.write("b.txt", "two\n").commit("feat: b");
    let moved = get(&app).await.expect("page").0;
    assert!(
        moved.contains("b.txt"),
        "a moved branch renders its new tree: {moved}"
    );
}

/// Guard class, as for the tree: the blame page is rendered once per (rev, commit, path), and a
/// moved branch is never served the old page.
#[tokio::test]
async fn blame_page_is_memoized_until_the_branch_moves() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = gitterrace_testkit::Fixture::init(dir.path())
        .write("a.txt", "one\n")
        .commit("feat: a");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = Arc::new(crate::app::App::configured(repo, index, &env));
    let get = |app: &Arc<crate::app::App>| {
        super::files::blame(
            State(Arc::clone(app)),
            Path(("main".into(), "a.txt".into())),
            axum::extract::Query(super::files::BlameQuery { from: None }),
        )
    };
    let first = get(&app).await.expect("page").0;
    assert!(first.contains("one"), "{first}");
    let head = app.repo.resolve("main").expect("head");
    let key = app.memo_key("blame", Some(&format!("main\0{head}\0a.txt\0")));
    app.memo_put(key, Arc::new("memo".into()));
    assert_eq!(
        get(&app).await.expect("page").0,
        "memo",
        "a repeat is not rebuilt"
    );
    let _f = f.write("a.txt", "two\n").commit("feat: b");
    let moved = get(&app).await.expect("page").0;
    assert!(
        moved.contains("two"),
        "a moved branch blames its new file: {moved}"
    );
}
