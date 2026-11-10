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

/// Walks of HEAD's counted sets so far, over the tables every unfiltered page of HEAD shares.
fn walks(app: &App) -> usize {
    let head = app.repo.resolve("HEAD").expect("head");
    let src = app.source(&head).expect("source").expect("ready");
    src.run(|s| {
        crate::fmt::with_rev("HEAD", || {
            let all: super::Src = Arc::clone(s) as super::Src;
            super::insights(app, &all, app.defaults().into_options()).walks()
        })
    })
    .expect("run")
}

/// Guard class: work per page, counted. A cold activity page walks its counted set once,
/// whatever sections it holds: red at 9 walks when commit types and code frequency each
/// streamed the set per reader. Second catch: a new section with a walk of its own.
#[tokio::test]
async fn a_cold_activity_page_walks_its_counted_set_once() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = gitterrace_testkit::Fixture::init(dir.path())
        .write("a/x.rs", "a\n")
        .commit("feat(a): one")
        .write("b/y.rs", "b\n")
        .commit("fix(b): two")
        .path()
        .to_owned();
    let repo = gitterrace_git::Repo::discover(&p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = Arc::new(App::new(repo, index));
    app.index
        .update(&app.repo, "HEAD", &|_| {})
        .expect("update");
    app.mark_ready();
    let router = crate::routes::router(Arc::clone(&app));
    let before = walks(&app);
    let (status, body) = gitterrace_testkit::get(&router, "/stats").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("id=\"types\""), "the types section rendered");
    assert_eq!(
        walks(&app) - before,
        1,
        "walks of the counted set for one cold /stats"
    );
}
