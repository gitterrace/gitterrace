use std::sync::Arc;

use askama::Template;

use crate::app::App;

#[derive(Template)]
#[template(source = "data", ext = "txt")]
struct Data;

/// The indexing page rendered just before the index turns ready is never memoized: the
/// next request after ready shows data, not a stale indexing page for the memo's minute.
#[tokio::test]
async fn the_indexing_page_is_never_memoized() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = gitterrace_testkit::Fixture::init(dir.path())
        .write("a.txt", "a\n")
        .commit("feat: a")
        .path()
        .to_owned();
    let repo = gitterrace_git::Repo::discover(&p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = Arc::new(App::new(repo, index));
    // The build has finished (the index generation moved) but the ready flag is not set
    // yet: a request in that window renders the indexing page under the final memo key.
    app.index
        .update(&app.repo, "HEAD", &|_| {})
        .expect("update");
    let first = super::cached(&app, "a", None, |_, _, _| Data)
        .await
        .expect("page");
    assert!(first.0.contains("data-progress"), "{}", first.0);
    app.mark_ready();
    let second = super::cached(&app, "a", None, |_, _, _| Data)
        .await
        .expect("page");
    assert_eq!(second.0, "data");
}

/// Guard class: concurrent requests for one cold memo key render the page once, the
/// rest wait and join it. Red when each request renders on its own (the warm-up and the
/// first visitor both built hotspots right after the index was ready, doubling its cost).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_for_one_cold_page_render_it_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().expect("tmp");
    let p = gitterrace_testkit::Fixture::init(dir.path())
        .write("a.txt", "a\n")
        .commit("feat: a")
        .path()
        .to_owned();
    let repo = gitterrace_git::Repo::discover(&p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = Arc::new(App::new(repo, index));
    app.index
        .update(&app.repo, "HEAD", &|_| {})
        .expect("update");
    app.mark_ready();
    let builds = Arc::new(AtomicUsize::new(0));
    let n = 8;
    let start = Arc::new(tokio::sync::Barrier::new(n));
    let mut tasks = Vec::new();
    for _ in 0..n {
        let (app, builds, start) = (Arc::clone(&app), Arc::clone(&builds), Arc::clone(&start));
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            super::cached(&app, "one", None, move |_, _, _| {
                builds.fetch_add(1, Ordering::SeqCst);
                // Long enough that every request arrives while the first renders.
                gitterrace_testkit::wait::pause_blocking(std::time::Duration::from_millis(200));
                Data
            })
            .await
            .expect("page")
            .0
        }));
    }
    for t in tasks {
        assert_eq!(t.await.expect("task"), "data");
    }
    assert_eq!(builds.load(Ordering::SeqCst), 1, "renders of one cold page");
    assert!(
        app.rendering.lock().expect("lock").is_empty(),
        "no render lock left behind"
    );
}
