//! Wiring tests for live updates, the watcher, the page shell, the action chords and the
//! overview.

#[cfg(test)]
mod worktree_tests;

use std::{sync::Arc, time::Duration};

use axum::{Router, body::Body, http::Request};
use gitterrace_config::Config;
use gitterrace_testkit::{git, page as get};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::app::Env;
use crate::wiring_tests::{app, repo, router, with};

/// Reads SSE text from a response body until `want` appears or the time is up.
async fn read_until(body: &mut Body, want: &str, within: Duration) -> Option<String> {
    let mut seen = String::new();
    let end = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < end {
        let left = end - tokio::time::Instant::now();
        match tokio::time::timeout(left, body.frame()).await {
            Ok(Some(Ok(f))) => {
                if let Some(d) = f.data_ref() {
                    seen.push_str(&String::from_utf8_lossy(d));
                    if seen.contains(want) {
                        return Some(seen);
                    }
                }
            }
            _ => break,
        }
    }
    None
}

/// Opens `uri` of the lone repository's event stream, under its prefix.
async fn open_events(r: &Router, uri: &str) -> axum::response::Response {
    let uri = format!("{}{uri}", gitterrace_testkit::base(r).await);
    r.clone()
        .oneshot(Request::get(uri).header("host", "localhost").body(Body::empty()).expect("req"))
        .await
        .expect("res")
}

#[tokio::test]
async fn events_deliver_each_kind_and_respect_the_stream_cap() {
    let dir = tempfile::tempdir().expect("tmp");
    let work = repo(dir.path(), "r");
    let server = router(
        &work,
        with(|c| {
            c.server.live_streams = 2;
            c.watch.rescan_secs = 5;
        }),
    );
    let first = open_events(&server, "/events").await;
    assert_eq!(first.status(), 200);
    assert_eq!(first.headers()["content-type"], "text/event-stream");
    let mut first = first.into_body();
    let second = open_events(&server, "/events").await;
    assert_eq!(second.status(), 200);
    let third = open_events(&server, "/events").await;
    assert_eq!(third.status(), 503, "server.live_streams = 2 refuses the third");
    drop(second);
    drop(third);
    // The watcher's first reading is its baseline: a commit made before it lands is not a
    // change. A loaded machine can delay that reading past any fixed sleep, so commit again
    // until the stream reports one, bounded by a deadline.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut n = 0;
    let got = loop {
        n += 1;
        std::fs::write(work.join("small.txt"), format!("needle {n}\n")).expect("write");
        git(&work, &["commit", "-qam", "fix: next"]);
        // The edit can show as a dirty chip first; the commit then follows as refs.
        if let Some(g) = read_until(&mut first, "\"refs\"", Duration::from_secs(2)).await {
            break g;
        }
        assert!(std::time::Instant::now() < deadline, "no change event, {n} tries");
    };
    assert!(got.contains("\"refs\""), "a commit moves the refs: {got}");
    // The slot of a closed stream is free again.
    assert_eq!(open_events(&server, "/events").await.status(), 200);
}

#[tokio::test]
async fn a_stream_that_missed_a_change_hears_it_on_connect() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let r = router(&p, Config::default());
    // A tab that rendered long before: its `v=1` is behind whatever the server holds now.
    gitterrace_testkit::wait::pause(Duration::from_millis(600)).await;
    let mut body = open_events(&r, "/events?v=1").await.into_body();
    let got = read_until(&mut body, "event: change", Duration::from_secs(2)).await;
    assert!(got.is_some_and(|g| g.contains("\"index\"")), "a stale version hears everything");
}

#[tokio::test]
async fn pages_carry_their_live_mode() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let r = router(&p, Config::default());
    let attr = |body: &str| {
        body.split("data-live=\"").nth(1).and_then(|s| s.split('"').next()).map(str::to_owned)
    };
    let (_, wt) = get(&r, "/worktree").await;
    assert_eq!(attr(&wt).as_deref(), Some("refresh"));
    assert!(wt.contains("/static/live.js"));
    for notice in ["/log/HEAD", "/refs", "/stats", "/"] {
        assert_eq!(attr(&get(&r, notice).await.1).as_deref(), Some("notice"), "{notice}");
    }
    let id = get(&r, "/log/HEAD").await.1;
    let id = id.split("/commit/").nth(1).and_then(|s| s.split('"').next()).expect("id").to_owned();
    let (_, commit) = get(&r, &format!("/commit/{id}")).await;
    assert_eq!(attr(&commit), None, "a pinned commit never listens");
    assert!(!commit.contains("/static/live.js"));
    let off = router(&p, with(|c| c.watch.enabled = false));
    let (_, page) = get(&off, "/worktree").await;
    assert_eq!(attr(&page), None, "watch.enabled = false opens no streams");
}

#[tokio::test]
async fn watch_off_runs_no_timer() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let a = app(&p, with(|c| c.watch.enabled = false));
    tokio::spawn(crate::app::keep_index_fresh_supervised(Arc::clone(&a)));
    let mut rx = a.changes.subscribe();
    gitterrace_testkit::wait::pause(Duration::from_millis(300)).await;
    git(&p, &["commit", "-q", "--allow-empty", "-m", "chore: empty"]);
    let got = tokio::time::timeout(Duration::from_millis(1500), rx.recv()).await;
    assert!(got.is_err(), "no change is announced with the watcher off");
}

#[tokio::test]
async fn working_tree_is_scanned_only_for_a_subscriber() {
    let _one = crate::wiring_fs_tests::SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let a = app(
        &p,
        with(|c| {
            c.watch.rescan_secs = 5;
        }),
    );
    tokio::spawn(crate::app::keep_index_fresh_supervised(Arc::clone(&a)));
    let mut rx = a.changes.subscribe();
    gitterrace_testkit::wait::pause(Duration::from_millis(400)).await;
    let kinds = |c: Arc<crate::live::Change>| c.kinds.clone();
    std::fs::write(p.join("small.txt"), "edited without a subscriber\n").expect("write");
    // The cheap dirty check may announce the flip, but never a full worktree scan.
    while let Ok(Ok(c)) = tokio::time::timeout(Duration::from_millis(900), rx.recv()).await {
        assert!(!kinds(c).contains(&"worktree"), "no worktree scan without a subscriber");
    }

    // The subscriber's first scan is its baseline; an edit before it finishes is the baseline.
    let before = scans(&a);
    let _wt = crate::live::WorktreeWatch::new(&a);
    assert!(
        scanned_after(&a, before, Duration::from_secs(20)).await,
        "a subscriber gets a baseline scan"
    );
    // Until the tree watch is up the refresher polls every second; the quiet check below is
    // about events, so it starts with the watch in place.
    crate::wiring_fs_tests::events_up(&a).await;
    while rx.try_recv().is_ok() {}
    std::fs::write(p.join("small.txt"), "edited with a subscriber\n").expect("write");
    assert!(worktree_event(&mut rx).await, "an edit is seen");
    std::fs::write(p.join("brand-new.txt"), "x\n").expect("write");
    assert!(worktree_event(&mut rx).await, "an untracked file is seen");
    // Late events from those writes may still rescan; once scans stop, nothing announces.
    settle(&a).await;
    while rx.try_recv().is_ok() {}
    let quiet = tokio::time::timeout(Duration::from_millis(900), rx.recv()).await;
    assert!(quiet.is_err(), "an untouched tree sends nothing");
}

fn scans(a: &crate::app::App) -> u64 {
    a.wt_scans.load(std::sync::atomic::Ordering::Acquire)
}

/// Waits until a scan finished after the count read `before`.
async fn scanned_after(a: &crate::app::App, before: u64, within: Duration) -> bool {
    gitterrace_testkit::wait::within(within, || async { scans(a) > before }).await
}

/// Returns once a second passes without a scan.
async fn settle(a: &crate::app::App) {
    let end = std::time::Instant::now() + Duration::from_secs(30);
    let mut last = u64::MAX;
    while scans(a) != last && std::time::Instant::now() < end {
        last = scans(a);
        gitterrace_testkit::wait::pause(Duration::from_secs(1)).await;
    }
}

/// True once a change naming the worktree arrives; other kinds (the dirty chip) are skipped.
async fn worktree_event(
    rx: &mut tokio::sync::broadcast::Receiver<Arc<crate::live::Change>>,
) -> bool {
    // The documented bound: the safety rescan (5 s here) plus margin.
    let end = tokio::time::Instant::now() + Duration::from_secs(10);
    while let Ok(Ok(c)) = tokio::time::timeout_at(end, rx.recv()).await {
        if c.kinds.contains(&"worktree") {
            return true;
        }
    }
    false
}

#[tokio::test]
async fn dirty_chip_follows_on_pages_without_the_worktree() {
    let _one = crate::wiring_fs_tests::SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    // A short safety rescan covers an event a loaded machine drops.
    let a = app(
        &p,
        with(|c| {
            c.watch.rescan_secs = 5;
        }),
    );
    tokio::spawn(crate::app::keep_index_fresh_supervised(Arc::clone(&a)));
    // A listener, but no page that shows the working tree.
    let mut rx = a.changes.subscribe();
    // The first check is the baseline; an edit before it would be the baseline itself.
    let first = || async { a.wt_scans.load(std::sync::atomic::Ordering::Relaxed) > 0 };
    gitterrace_testkit::wait::until("the baseline check", first).await;
    while rx.try_recv().is_ok() {}
    let mut next = async |want: bool| {
        loop {
            let c = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("event")
                .expect("open");
            if c.dirty == want {
                return;
            }
        }
    };
    std::fs::write(p.join("small.txt"), "edited\n").expect("write");
    next(true).await;
    git(&p, &["checkout", "-q", "--", "small.txt"]);
    next(false).await;
    assert_eq!(a.wt_watchers.load(std::sync::atomic::Ordering::Acquire), 0);
}

/// A refused actions.toml is in the page's first HTML, so the note never arrives late and
/// shifts the page down.
#[cfg(unix)]
#[tokio::test]
async fn a_refused_actions_file_is_in_the_first_paint() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let conf = dir.path().join("conf");
    std::fs::create_dir(&conf).expect("dir");
    std::fs::set_permissions(&conf, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    let file = conf.join("actions.toml");
    std::fs::write(
        &file,
        "version = 1\n[[action]]\nid = \"x\"\nlabel = \"X\"\nkinds = [\"page\"]\ncommand = \
        [\"true\"]\nenv = [\"\"]\n",
    )
    .expect("write");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let env = Env::new(Config::default());
    let (r, _) = crate::build(&p, None, Some(&file), &env).expect("build");
    let (code, html) = get(&r, "/").await;
    assert_eq!(code, 200);
    let strip = html
        .split("id=\"actnote\"")
        .nth(1)
        .and_then(|s| s.split("</div></div>").next())
        .expect("strip rendered");
    assert!(!strip.starts_with(" hidden"), "{strip}");
    assert!(strip.contains("actions.toml not loaded"), "{strip}");
}

/// `links.style` decides what Copy permalink copies: the page tells the scripts which form.
#[tokio::test]
async fn links_style_switches_copy_permalink() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, body) = get(&router(&p, Config::default()), "/blob/HEAD/small.txt").await;
    assert!(body.contains("data-link=\"gitterrace\""), "default gitterrace");
    let go = with(|c| c.links.style = gitterrace_types::LinkStyle::Go);
    let (_, body) = get(&router(&p, go), "/blob/HEAD/small.txt").await;
    assert!(body.contains("data-link=\"go\""), "links.style = go");
    let js = include_str!("../static/actions.js");
    assert!(
        js.contains("dataset.link") && js.contains("\"gitterrace://\""),
        "actions.js reads the style"
    );
}

/// "a" then an action's key fires the action from the page, and the `?` sheet lists the chord.
/// The keys come from the loader-validated menu the page already embeds.
#[cfg(unix)]
#[tokio::test]
async fn action_chords_are_wired_and_listed_in_the_help_sheet() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let conf = dir.path().join("conf");
    std::fs::create_dir(&conf).expect("dir");
    std::fs::set_permissions(&conf, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    let file = conf.join("actions.toml");
    std::fs::write(
        &file,
        "version = 1\n[[action]]\nid = \"x\"\nlabel = \"X\"\nkinds = [\"page\"]\ncommand = \
        [\"/bin/echo\"]\nkey = \"x\"\n",
    )
    .expect("write");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let env = Env::new(Config::default());
    let (r, _) = crate::build(&p, None, Some(&file), &env).expect("build");
    let (_, html) = get(&r, "/").await;
    assert!(
        html.contains("&#34;key&#34;:&#34;x&#34;") || html.contains("\"key\":\"x\""),
        "menu carries the key"
    );
    let nav = include_str!("../static/nav.js");
    assert!(nav.contains("\"a \" + a.key"), "the ? sheet lists `a <key>`");
}

#[tokio::test]
async fn commit_page_links_only_the_targets_with_a_lookup() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let file = dir.path().join("actions.toml");
    std::fs::write(
        &file,
        "version = 1\n\
         [[send]]\nid = \"a\"\nlabel = \"Alpha\"\nurl = \"mailto:?subject={title}\"\n\
         lookup = \"https://alpha.test/s?q={sha}&s={short}\"\n\
         [[send]]\nid = \"b\"\nlabel = \"Beta\"\nurl = \"mailto:?subject={title}\"\n",
    )
    .expect("write");
    let (app, _) =
        crate::build(&p, None, Some(&file), &Env::new(Config::default())).expect("build");
    let (_, log) = get(&app, "/log/HEAD").await;
    let id = log
        .split("/commit/")
        .nth(1)
        .and_then(|s| s.split(['"', '#']).next())
        .expect("a commit link")
        .to_owned();
    let (_, page) = get(&app, &format!("/commit/{id}")).await;
    assert!(page.contains("Find this commit in"), "{page}");
    assert!(page.contains(&format!("https://alpha.test/s?q={id}&#38;s={}", &id[..7])), "{page}");
    assert!(page.contains(">alpha.test</a>") && !page.contains("Alpha") && !page.contains("Beta"));
    let (_, plain) = get(&router(&p, Config::default()), &format!("/commit/{id}")).await;
    assert!(!plain.contains("Find this commit in"));
}

/// The overview counts the checked-out branch against its upstream's tracking ref, read from
/// local refs, and says nothing when the branch has no upstream.
#[tokio::test]
async fn overview_shows_how_far_behind_its_upstream_the_branch_is() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = gitterrace_testkit::Fixture::init(&dir.path().join("r"))
        .write("a.txt", "a\n")
        .commit("feat: one")
        .write("a.txt", "b\n")
        .commit("feat: two")
        .write("a.txt", "c\n")
        .commit("feat: three")
        .path()
        .to_owned();
    let (_, plain) = get(&router(&p, Config::default()), "/").await;
    assert!(!plain.contains("behind") && !plain.contains("up to date"), "{plain}");
    let tip = git(&p, &["rev-parse", "HEAD"]);
    let branch = git(&p, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let (tip, branch) = (tip.trim(), branch.trim());
    git(&p, &["update-ref", &format!("refs/remotes/origin/{branch}"), tip]);
    git(&p, &["remote", "add", "origin", "https://example.invalid/r.git"]);
    git(&p, &["config", &format!("branch.{branch}.remote"), "origin"]);
    let merge = format!("refs/heads/{branch}");
    git(&p, &["config", &format!("branch.{branch}.merge"), &merge]);
    let (_, even) = get(&router(&p, Config::default()), "/").await;
    assert!(even.contains("up to date with origin/"), "{even}");
    git(&p, &["reset", "-q", "--hard", "HEAD~2"]);
    let (_, two) = get(&router(&p, Config::default()), "/").await;
    assert!(two.contains("<b>2</b> behind origin/"), "{two}");
}
