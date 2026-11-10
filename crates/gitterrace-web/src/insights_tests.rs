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

/// Guard class: one table set per tip. The cold overview after an insights page builds no
/// derived tables of its own: red at 1 when its totals opened a fresh `Insights` over HEAD.
/// Second catch: any page that counts HEAD's full set beside the shared tables.
#[tokio::test]
async fn the_overview_reuses_the_shared_tables_of_head() {
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
    let router = crate::routes::router(Arc::clone(&app));
    let (status, _) = gitterrace_testkit::get(&router, "/stats").await;
    assert_eq!(status, 200);
    let builds = || {
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        src.run(|s| gitterrace_index::table_builds(&**s))
            .expect("run")
    };
    let before = builds();
    let (status, body) = gitterrace_testkit::get(&router, "/").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(builds() - before, 0, "table sets built for the overview");
}

/// Guard class: what the warm-up builds is what the pages read. After the before-ready
/// pass, a cold activity page and overview build no table set of their own.
#[tokio::test]
async fn pages_after_the_warm_up_build_no_tables() {
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
    let head = app.repo.resolve("HEAD").expect("head");
    super::warm_shared(&app, &head);
    app.mark_ready();
    let builds = || {
        let src = app.source(&head).expect("source").expect("ready");
        src.run(|s| gitterrace_index::table_builds(&**s))
            .expect("run")
    };
    let before = builds();
    let router = crate::routes::router(Arc::clone(&app));
    for page in ["/stats", "/"] {
        let (status, _) = gitterrace_testkit::get(&router, page).await;
        assert_eq!(status, 200, "{page}");
    }
    assert_eq!(builds() - before, 0, "table sets built after the warm-up");
}

/// Guard class: a count a shared table holds is read, not walked again. After the
/// before-ready pass a cold overview reads no file rows: red at 1 when its all-time totals
/// streamed every row for the distinct files. Second catch: any whole-history reducer
/// that asks for rows a table already summarizes.
#[tokio::test]
async fn a_cold_overview_reads_no_file_rows() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = gitterrace_testkit::Fixture::init(dir.path())
        .write("a.txt", "a\n")
        .commit("feat: a")
        .write("b.txt", "b\n")
        .commit("feat: b")
        .path()
        .to_owned();
    let repo = gitterrace_git::Repo::discover(&p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = Arc::new(App::new(repo, index));
    app.index
        .update(&app.repo, "HEAD", &|_| {})
        .expect("update");
    let head = app.repo.resolve("HEAD").expect("head");
    super::warm_shared(&app, &head);
    app.mark_ready();
    let rows = || {
        let src = app.source(&head).expect("source").expect("ready");
        src.run(|s| {
            crate::fmt::with_rev("HEAD", || {
                let all: super::Src = Arc::clone(s) as super::Src;
                super::insights(&app, &all, app.defaults().into_options()).row_walks()
            })
        })
        .expect("run")
    };
    let before = rows();
    let router = crate::routes::router(Arc::clone(&app));
    let (status, body) = gitterrace_testkit::get(&router, "/").await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("<b>2</b> files touched"),
        "the file count is still shown"
    );
    assert_eq!(rows() - before, 0, "row walks of a cold overview");
}
