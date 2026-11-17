//! A server's index update runs in a child process, so the build's peak (allocator metadata,
//! worker stacks) dies with it instead of staying with the server.
//!
//! The child is this test executable started again with the same arguments, as the binary
//! starts itself: the test calls `index_child` first, as `main` does, so the child runs the
//! update and exits there.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Index worker threads started in this process.
static WORKERS: AtomicUsize = AtomicUsize::new(0);

fn count(on: bool) {
    if on {
        WORKERS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Guard class: any index work done on the server's own threads, whether a new caller of
/// `Index::update` or a build step moved out of the child, starts a worker here.
#[tokio::test(flavor = "multi_thread")]
async fn the_server_builds_its_index_without_a_worker_of_its_own() {
    gitterrace_git::set_worker_hook(count);
    gitterrace_web::index_child();
    let dir = tempfile::tempdir().expect("tmp");
    let p = gitterrace_testkit::Fixture::init(&dir.path().join("r"))
        .write("a.txt", "one\n")
        .commit("feat: one")
        .write("a.txt", "two\n")
        .commit("fix: two")
        .path()
        .to_owned();
    let router = gitterrace_web::app_router(&p).expect("router");
    let body = gitterrace_testkit::wait::page_until(&router, "/api/progress", |b| {
        b.contains("\"ready\":true")
    })
    .await;
    assert!(
        body.contains("\"name\":\"stream\""),
        "steps of the child: {body}"
    );
    assert_eq!(
        WORKERS.load(Ordering::Relaxed),
        0,
        "index workers ran in the server"
    );
}
