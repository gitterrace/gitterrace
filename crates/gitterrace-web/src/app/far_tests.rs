//! The reindex notice of a revision too far from the indexed branch.

use std::sync::Arc;

use gitterrace_testkit::git;

use crate::app::App;

/// A repository whose `far` branch has more commits off `main` than the reach view reads,
/// written with one fast-import: thousands of `git commit` calls take minutes.
fn far_branch_app() -> (tempfile::TempDir, Arc<App>) {
    use std::fmt::Write as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    let mut s = String::new();
    let far = gitterrace_index::feed::FRESH_CAP + 5;
    for n in 0..=far {
        let (branch, from) = if n == 0 { ("main", None) } else { ("far", Some(n)) };
        let t = 1_700_000_000 + n * 60;
        writeln!(
            s,
            "commit refs/heads/{branch}\nmark :{}\nauthor A <a@x.io> {t} +0000\n\
             committer A <a@x.io> {t} +0000\ndata 8\nfeat: {:02}",
            n + 1,
            n % 100
        )
        .expect("write");
        if let Some(f) = from {
            writeln!(s, "from :{f}").expect("write");
        }
        let body = format!("line {n}\n");
        write!(s, "M 100644 inline a.txt\ndata {}\n{body}\n", body.len()).expect("write");
    }
    gitterrace_testkit::git_stdin(path, &["fast-import", "--quiet"], s.as_bytes());
    git(path, &["checkout", "-q", "main"]);
    let app = crate::tests::ready_app(path);
    (dir, app)
}

/// The reindex notice of a far revision comes from the selection that found it far, not from
/// a cache read after it: an idle release (a far page never counts as use) or a generation
/// change between the two turned the notice into the indexing page.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_far_revision_keeps_its_notice_through_an_idle_release() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_dir, app) = far_branch_app();
    let stop = Arc::new(AtomicBool::new(false));
    let releaser = {
        let (app, stop) = (Arc::clone(&app), Arc::clone(&stop));
        std::thread::spawn(move || {
            // This app's caches only, as the release clears them: the release also drops the
            // process-wide shared tables, which other tests running alongside hold.
            while !stop.load(Ordering::Relaxed) {
                app.caches.clear_all();
                std::thread::yield_now();
            }
        })
    };
    let mut missing = 0;
    for _ in 0..5 {
        let (status, body) = crate::tests::get(&app, "/stats?rev=far").await;
        assert_eq!(status, 200);
        if !body.contains("need a reindex") {
            missing += 1;
        }
    }
    stop.store(true, Ordering::Relaxed);
    releaser.join().expect("releaser");
    assert_eq!(missing, 0, "far pages without the reindex notice, of 5");
}
