//! Tests for the shared state: memo keys, caches, idle release and the refresher.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gitterrace_testkit::git;

use super::{App, LastCommit};

#[test]
fn junk_and_order_do_not_make_new_memo_keys() {
    let canon = |q| App::canonical_query(Some(q));
    assert_eq!(canon("x=1&q=type:fix"), canon("q=type:fix&x=2&y"));
    assert_eq!(canon("period=1y&q=type:fix"), canon("q=type:fix&period=1y"));
    assert_ne!(canon("q=type:fix"), canon("q=type:feat"));
    assert_eq!(canon("type=fix&author=a"), App::canonical_query(None));
}

#[test]
fn memo_key_moves_with_head_and_index_and_the_source_follows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    std::fs::write(path.join("a.txt"), "1\n").expect("write");
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "feat: one"]);
    let app = crate::tests::ready_app(path);
    let head = app.repo.resolve("HEAD").expect("head");
    let key = app.memo_key("t", Some("q=type:fix"));
    assert_ne!(key, app.memo_key("t", Some("q=type:feat")));
    app.memo_put(key.clone(), "page");
    assert!(app.memo_get(&key).is_some());
    let first = crate::tests::unchecked_src(&app, &head);
    assert_eq!(first.len(), 1);

    std::fs::write(path.join("a.txt"), "2\n").expect("write");
    git(path, &["commit", "-qam", "fix: two"]);
    let moved = app.memo_key("t", Some("q=type:fix"));
    assert_ne!(key, moved, "moving HEAD must change the memo key");
    assert!(app.memo_get(&moved).is_none());
    app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
    app.mark_ready();
    let head = app.repo.resolve("HEAD").expect("head");
    let second = crate::tests::unchecked_src(&app, &head);
    assert_eq!(second.len(), 2, "the source must follow the new tip");
}

/// Overview totals are an all-history pass (about 400 ms per request on rust-lang/rust
/// before the memo). A repeat at the same tip must not read the records again: it is
/// handed none here, so any recomputation counts zero commits.
#[test]
fn overview_totals_are_computed_once_per_tip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    std::fs::write(path.join("a.txt"), "1\n").expect("write");
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "feat: one"]);
    let app = crate::tests::ready_app(path);
    let opts = || app.defaults().into_options();
    let head = app.repo.resolve("HEAD").expect("head");
    let src = crate::tests::unchecked_src(&app, &head);
    assert_eq!(app.totals(&head, &*src, opts()).counted.commits, 1);
    let empty = gitterrace_index::feed::MemorySource::new(&[]);
    assert_eq!(app.totals(&head, &empty, opts()).counted.commits, 1, "repeat must hit the memo");

    std::fs::write(path.join("a.txt"), "2\n").expect("write");
    git(path, &["commit", "-qam", "fix: two"]);
    app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
    let head = app.repo.resolve("HEAD").expect("head");
    let src = crate::tests::unchecked_src(&app, &head);
    assert_eq!(app.totals(&head, &*src, opts()).counted.commits, 2, "a new tip recomputes");
}

#[test]
fn racing_callers_share_one_source() {
    let (_dir, app, head) = crate::tests::one_commit_app();
    let sets: Vec<_> = (0..8)
        .map(|_| {
            let (a, h) = (Arc::clone(&app), head.clone());
            std::thread::spawn(move || a.source(&h).expect("source").expect("ready").unchecked())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().expect("join"))
        .collect();
    assert!(sets.iter().all(|s| s.len() == 1), "every caller reads the set");
    // Racers may each open the set (it is a pread view, cheap); once one is kept, all share it.
    let kept = crate::tests::unchecked_src(&app, &head);
    let again = crate::tests::unchecked_src(&app, &head);
    assert!(Arc::ptr_eq(&kept, &again), "a kept source is shared");
}

#[test]
fn alternating_selected_commits_each_read_the_segment_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    std::fs::write(path.join("a.txt"), "1\n").expect("write");
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "feat: one"]);
    let first = git(path, &["rev-parse", "HEAD"]);
    std::fs::write(path.join("a.txt"), "2\n").expect("write");
    git(path, &["commit", "-qam", "fix: two"]);
    let app = crate::tests::ready_app(path);
    let head = app.repo.resolve("HEAD").expect("head");
    for _ in 0..3 {
        assert_eq!(crate::tests::unchecked_src(&app, &head).len(), 2);
        assert_eq!(app.source(&first).expect("source").expect("ready").unchecked().len(), 1);
    }
}

/// Nothing may keep the insights source and the tables over it alive once nobody browses:
/// not the source slot, the shared insights tables or any cache added later. The test holds
/// only a weak reference, so whatever pins the source turns it red; a page in flight keeps its
/// own `Arc` and the next page opens the set again.
#[test]
fn an_idle_app_lets_go_of_its_source_and_opens_it_again_on_use() {
    let (_dir, app, head) = crate::tests::one_commit_app();
    // Everything the pages build over the source, as the warm-up does.
    let src = crate::tests::unchecked_src(&app, &head);
    app.chain(&head, &*src, &crate::languages::path_filter(&app, &head));
    crate::fmt::with_rev(&head, || {
        let _ins = crate::insights::insights(&app, &src, gitterrace_index::Options::default());
    });
    let weak = Arc::downgrade(&src);
    drop(src);
    assert!(app.source_held());
    let tables = || app.caches.tables.len();
    assert_eq!(tables(), 1);
    assert!(!app.release_idle(Duration::from_secs(3600)), "in use");
    assert!(weak.upgrade().is_some(), "kept while browsed");
    assert!(app.release_idle(Duration::ZERO));
    assert!(weak.upgrade().is_none(), "an idle app still holds its source");
    assert_eq!(tables(), 0, "an idle app still holds the tables built over it");
    assert!(!app.source_held(), "an idle app still holds its insights source");
    let again = crate::tests::unchecked_src(&app, &head);
    assert_eq!(again.len(), 1);
}

/// A page rendered while a read was failing (another request's sidebar, built just before its
/// own read failed) is never served once the rebuild starts. The rebuilt cache gets the same
/// index generation, so a memo entry would match it and show the broken page again.
#[test]
fn an_unreadable_cache_drops_every_memoized_page() {
    let (_dir, app, _head) = crate::tests::one_commit_app();
    let page = app.memo_key("overview", None);
    app.memo_put(page.clone(), "built over a failing read");
    assert!(app.memo_get(&page).is_some());
    app.unreadable_now().expect("rebuild");
    assert!(app.memo_get(&page).is_none(), "a page memo survived the rebuild");
}

/// Every per-request cache is empty after an idle release, whichever page filled it. The
/// caches live in one struct whose clear names every field, so a new one cannot be left
/// out; this checks the release reaches that clear and that each filled cache empties.
#[test]
fn an_idle_release_empties_every_cache() {
    let (_dir, app, head) = crate::tests::one_commit_app();
    let src = crate::tests::unchecked_src(&app, &head);
    app.chain(&head, &*src, &crate::languages::path_filter(&app, &head));
    let _ = app.file_paths(&head).expect("files");
    let _ = app.last_commits(&head, "").expect("last commits");
    let _ = app.ignore_set(&head);
    let _ = app.head_lines(&head);
    drop(src);
    assert!(app.caches.clear_all(), "the pages above filled some cache");
    let _ = app.source(&head).expect("source");
    let _ = app.file_paths(&head).expect("files");
    assert!(app.release_idle(Duration::ZERO));
    assert!(!app.caches.clear_all(), "a cache survived the idle release");
}

/// The caches that live outside the per-request set empty on the idle release too: the line
/// blame the git handle keeps and the highlighted files the code view shares across the
/// process. Each holds up to its share of the cache budget, and an idle server kept both.
/// The code memo is shared with other tests, so only its state after the release is checked.
#[test]
fn an_idle_release_empties_blame_and_highlighted_code() {
    let (_dir, app, head) = crate::tests::one_commit_app();
    let _ = app.repo.blame(&head, "a.rs").expect("blame");
    assert!(app.repo.blame_is_cached(&head, "a.rs"), "the blame was kept");
    let r = crate::views::Render::test(1 << 20);
    let text = "fn released_on_idle() {}\n";
    let _ = crate::views::code_html(&r, "idle.rs", text);
    assert!(app.release_idle(Duration::ZERO));
    assert!(!app.repo.blame_is_cached(&head, "a.rs"), "the blame survived the idle release");
    assert!(
        !crate::views::code_is_cached(&r, "idle.rs", text),
        "a highlighted file survived the idle release"
    );
}

/// Warm-up work alone never postpones the release: a server that only warmed itself is
/// idle. A visitor's read does postpone it. The last use is set back by a day instead of
/// waited for, so no scheduling delay moves either answer.
#[test]
fn warm_up_reads_do_not_delay_the_release() {
    const IDLE: Duration = Duration::from_secs(3600);
    let (_dir, app, head) = crate::tests::one_commit_app();
    let set_back = || {
        let past =
            Instant::now().checked_sub(Duration::from_hours(24)).expect("a clock past one day");
        *app.records_used.lock().expect("lock") = past;
    };
    set_back();
    {
        let _warm = app.warming();
        let src = crate::tests::unchecked_src(&app, &head);
        app.chain(&head, &*src, &crate::languages::path_filter(&app, &head));
    }
    assert!(app.release_idle(IDLE), "the warm-up kept the caches as if a visitor had read them");
    set_back();
    let _ = crate::tests::unchecked_src(&app, &head);
    assert!(!app.release_idle(IDLE), "a visitor's read did not count as use");
}

/// A history with renames, a dir rename, deletes, an untouched-path merge, a merge that
/// takes one side, and an evil merge that matches neither parent.
fn tangled() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let mut day = 0;
    let mut commit = |msg: &str| {
        day += 1;
        let date = format!("2024-01-{day:02}T12:00:00Z");
        gitterrace_testkit::git_at(p, &date, &["add", "-A"]);
        gitterrace_testkit::git_at(p, &date, &["commit", "-q", "--allow-empty", "-m", msg]);
        date
    };
    let w = |f: &str, body: &str| {
        let f = p.join(f);
        std::fs::create_dir_all(f.parent().expect("parent")).expect("mkdir");
        std::fs::write(f, body).expect("write");
    };
    git(p, &["init", "-q", "-b", "main"]);
    w("d/a", "1\n");
    w("d/b", "1\n");
    w("e/c", "1\n");
    commit("base");
    w("d/a", "2\n");
    commit("a two");
    git(p, &["checkout", "-q", "-b", "side"]);
    w("d/b", "side\n");
    w("e/c", "side\n");
    commit("side b");
    git(p, &["checkout", "-q", "main"]);
    w("d/a", "3\n");
    commit("a three");
    let date = commit("unused");
    gitterrace_testkit::git_at(p, &date, &["merge", "-q", "--no-ff", "--no-commit", "side"]);
    w("d/b", "evil\n");
    commit("evil merge");
    git(p, &["mv", "d/a", "d/a2"]);
    commit("rename a");
    git(p, &["checkout", "-q", "-b", "other", "HEAD~3"]);
    w("e/c", "other\n");
    commit("other c");
    git(p, &["checkout", "-q", "main"]);
    let date = commit("empty");
    let theirs = ["-X", "theirs", "-m", "merge other", "other"];
    let merge = [&["merge", "-q", "--no-ff"][..], &theirs].concat();
    gitterrace_testkit::git_at(p, &date, &merge);
    git(p, &["mv", "e", "f"]);
    commit("move e");
    std::fs::remove_file(p.join("d/b")).expect("rm");
    commit("drop b");
    // A side branch whose commit is dated long before its parent (rust has these), then
    // merged: a date-ordered walk meets it after commits it should have hidden.
    git(p, &["checkout", "-q", "-b", "skew"]);
    w("d/a2", "skew\n");
    let old = "2023-06-01T12:00:00Z";
    gitterrace_testkit::git_at(p, old, &["commit", "-qam", "skewed a"]);
    git(p, &["checkout", "-q", "main"]);
    w("f/c", "late\n");
    commit("late c");
    let date = commit("unused2");
    let merge = ["merge", "-q", "--no-ff", "-m", "merge skew", "skew"];
    gitterrace_testkit::git_at(p, &date, &merge);
    let app = crate::tests::ready_app(p);
    (dir, app)
}

/// The indexed path log and last-commit walk must list exactly what the full history walk
/// lists, for files, dirs, renamed, moved, deleted and merged paths, at every commit.
#[test]
fn indexed_path_history_matches_the_walk() {
    let (dir, app) = tangled();
    let all = git(dir.path(), &["rev-list", "HEAD"]);
    let paths = ["d", "d/a", "d/a2", "d/b", "e", "e/c", "f", "f/c", "nope"];
    for tip in all.lines() {
        for path in paths {
            let walked: Vec<String> = app
                .repo
                .log(tip, Some(path), 0, usize::MAX)
                .expect("log")
                .into_iter()
                .map(|c| c.id)
                .collect();
            let log = app.path_log(tip, path).expect("path log").expect("ready");
            assert!(log.complete);
            assert_eq!(log.ids, walked, "path {path} at {tip}");
            // Users compare against git itself: its default history simplification.
            let real = git(dir.path(), &["log", "--format=%H", tip, "--", path]);
            let real: Vec<String> = real.lines().map(str::to_owned).collect();
            assert_eq!(walked, real, "git log -- {path} at {tip}");
        }
        for d in ["", "d", "e", "f"] {
            let names =
                |v: Vec<LastCommit>| -> Vec<_> { v.into_iter().map(|l| (l.name, l.id)).collect() };
            let walked =
                gitterrace_git::last_commits(&app.repo, tip, d, Duration::MAX).expect("walk").0;
            let indexed = app.last_commits(tip, d).expect("indexed");
            let indexed = names((*indexed).clone());
            assert_eq!(indexed, names(walked), "dir {d:?} at {tip}");
        }
    }
}

/// A path's history is answered once per (tip, path, index generation): a repeat must not
/// filter history again. Seen red with the `path_logs` put removed.
#[test]
fn path_log_is_memoized_per_tip_and_generation() {
    let (dir, app) = tangled();
    let head = app.repo.resolve("HEAD").expect("head");
    let first = app.path_log(&head, "d/b").expect("log").expect("ready");
    let again = app.path_log(&head, "d/b").expect("log").expect("ready");
    assert!(Arc::ptr_eq(&first, &again), "a repeat must hit the memo");
    std::fs::write(dir.path().join("d/a2"), "4\n").expect("write");
    git(dir.path(), &["commit", "-qam", "four"]);
    app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
    let head2 = app.repo.resolve("HEAD").expect("head");
    let moved = app.path_log(&head2, "d/a2").expect("log").expect("ready");
    let newest = moved.ids.first();
    assert_eq!(newest, Some(&head2), "a new tip lists its own commit");
}

/// A commit whose tree cannot be read makes the indexed log give up (`complete` false, no
/// ids), so the route walks as before instead of serving a list with a gap.
#[test]
fn a_missing_tree_cuts_the_path_log_to_a_correct_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    for n in 0..6 {
        std::fs::write(p.join("f"), format!("{n}\n")).expect("write");
        std::fs::write(p.join("g"), format!("{n}\n")).expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", &format!("c{n}")]);
    }
    let app = crate::tests::ready_app(p);
    let head = app.repo.resolve("HEAD").expect("head");
    let walked = app.repo.log(&head, Some("f"), 0, 3).expect("log").into_iter().len();
    let spec = format!("HEAD~4^{}tree{}", '{', '}');
    let tree = git(p, &["rev-parse", &spec]);
    let (dir2, file) = tree.trim().split_at(2);
    std::fs::remove_file(p.join(".git/objects").join(dir2).join(file)).expect("rm tree");
    let log = app.path_log(&head, "f").expect("log").expect("ready");
    assert!(!log.complete, "the cut must be reported");
    assert!(log.ids.is_empty(), "no partial list is kept");
    assert_eq!(walked, 3, "the walk still answers the newer commits");
}

#[test]
fn last_commit_cache_follows_history_not_tree_and_skips_cut_walks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    let commit = |msg: &str, date: &str| {
        gitterrace_testkit::git_at(path, date, &["add", "-A"]);
        gitterrace_testkit::git_at(path, date, &["commit", "-q", "-m", msg]);
        git(path, &["rev-parse", "HEAD"])
    };
    git(path, &["init", "-q", "-b", "main"]);
    std::fs::create_dir(path.join("d")).expect("mkdir");
    std::fs::write(path.join("d/f"), "1\n").expect("write");
    commit("feat: base", "2024-01-01T12:00:00Z");
    git(path, &["checkout", "-q", "-b", "side"]);
    std::fs::write(path.join("d/f"), "2\n").expect("write");
    let on_side = commit("feat: side", "2024-02-01T12:00:00Z");
    git(path, &["checkout", "-q", "main"]);
    std::fs::write(path.join("d/f"), "2\n").expect("write");
    let on_main = commit("feat: main", "2024-03-01T12:00:00Z");
    let repo = gitterrace_git::Repo::discover(path).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = App::new(repo, index);
    let cut = app.last_commits_within(&on_main, "d", std::time::Duration::ZERO).expect("cut");
    assert!(cut.is_empty());
    let main = app.last_commits(&on_main, "d").expect("main");
    assert_eq!(main.first().map(|c| c.id.as_str()), Some(on_main.as_str()));
    let side = app.last_commits(&on_side, "d").expect("side");
    assert_eq!(side.first().map(|c| c.id.as_str()), Some(on_side.as_str()));
}
