//! Every setting the server honours has a test here that changes it and sees behaviour change.

use std::{path::Path, process::Command, sync::Arc, time::Duration};

use axum::{Router, body::Body, http::Request};
use gitterrace_config::Config;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::app::{App, Env};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=T", "-c", "user.email=t@e.x"])
        .args(args)
        .current_dir(dir)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "git {args:?}");
}

/// A repository with a small file, a 3 KiB file and two commits.
fn repo(root: &Path, name: &str) -> std::path::PathBuf {
    let p = root.join(name);
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("small.txt"), "needle one\n").expect("write");
    std::fs::write(p.join("mid.txt"), "needle\n".repeat(450)).expect("write");
    std::fs::write(p.join("yarn.lock"), "lock\n").expect("write");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "feat: first"]);
    std::fs::write(p.join("small.txt"), "needle two\n").expect("write");
    git(&p, &["commit", "-qam", "fix: second"]);
    p
}

fn with(f: impl FnOnce(&mut Config)) -> Config {
    let mut c = Config::default();
    f(&mut c);
    c.validate().expect("valid");
    c
}

async fn get(r: &Router, uri: &str) -> (u16, String) {
    let res = r
        .clone()
        .oneshot(
            Request::get(uri)
                .header("host", "localhost")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("res");
    let s = res.status().as_u16();
    let b = res.into_body().collect().await.expect("body").to_bytes();
    (s, String::from_utf8_lossy(&b).into_owned())
}

fn router(p: &Path, cfg: Config) -> Router {
    crate::app_router_with(p, cfg).expect("router")
}

fn app(p: &Path, cfg: Config) -> Arc<App> {
    let repo = gitterrace_git::Repo::discover(p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    Arc::new(App::configured(repo, index, &Env::new(cfg)))
}

#[tokio::test]
async fn limits_text_kb_and_raw_mb_change_what_a_file_shows() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, body) = get(&router(&p, Config::default()), "/blob/HEAD/mid.txt").await;
    assert!(body.contains("class=\"code"), "3 KiB shows by default");
    let small = with(|c| c.limits.text_kb = 1);
    let (_, body) = get(&router(&p, small), "/blob/HEAD/mid.txt").await;
    assert!(
        body.contains("File too large to display"),
        "text_kb = 1 refuses it"
    );
    let (_, body) = get(
        &router(&p, with(|c| c.limits.text_kb = 1)),
        "/blame/HEAD/mid.txt",
    )
    .await;
    assert!(body.contains("too large"));

    let big = "x\n".repeat(600 * 1024);
    std::fs::write(p.join("big.txt"), &big).expect("write");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-qm", "chore: big"]);
    let (s, _) = get(&router(&p, Config::default()), "/raw/HEAD/big.txt").await;
    assert_eq!(s, 200);
    let tiny = with(|c| c.limits.raw_mb = 1);
    assert_eq!(
        get(&router(&p, tiny), "/raw/HEAD/big.txt").await.0,
        413,
        "1.2 MB is over 1 MiB"
    );
}

#[tokio::test]
async fn limits_search_matches_caps_the_results() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, all) = get(&router(&p, Config::default()), "/search?q=needle").await;
    assert!(!all.contains("Showing the first results only"));
    let capped = with(|c| c.limits.search_matches = 3);
    let (_, body) = get(&router(&p, capped), "/search?q=needle").await;
    assert!(body.contains("Showing the first results only"), "{body}");
}

#[tokio::test]
async fn limits_diff_fold_lines_folds_a_big_file_diff() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, head) = get(&router(&p, Config::default()), "/log/HEAD").await;
    let id = head
        .split("/commit/")
        .nth(1)
        .and_then(|s| s.split(['"', '#']).next())
        .expect("a commit link")
        .to_owned();
    let (_, open) = get(&router(&p, Config::default()), &format!("/commit/{id}")).await;
    assert!(!open.contains("data-load-diff"));
    let fold = with(|c| c.limits.diff_fold_lines = 1);
    let (_, folded) = get(&router(&p, fold), &format!("/commit/{id}")).await;
    assert!(
        folded.contains("data-load-diff"),
        "two changed lines fold at 1"
    );
}

#[tokio::test]
async fn display_theme_and_dates_change_the_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, body) = get(&router(&p, Config::default()), "/log/HEAD").await;
    assert!(!body.contains("data-theme") && body.contains("just now"));
    let dark = with(|c| {
        c.display.theme = "dark".into();
        c.display.dates = "absolute".into();
    });
    let (_, body) = get(&router(&p, dark), "/log/HEAD").await;
    assert!(body.contains("<html lang=\"en\" data-theme=\"dark\">"));
    assert!(
        !body.contains("just now") && !body.contains("ago"),
        "dates are absolute"
    );
}

#[test]
fn insights_defaults_follow_the_config_and_the_repo_override() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "alpha");
    let none = app(&p, Config::default()).defaults();
    assert!(!none.bots && !none.generated && none.exclude.is_empty() && none.period.is_none());

    let cfg = gitterrace_config::parse(
        "[insights]\nbots = true\ngenerated = true\nperiod = \"3m\"\nexclude = [\"*.lock\"]\n\n[repo.alpha]\nexclude = [\"docs/**\"]\n[repo.beta]\nexclude = [\"nope\"]\n",
    )
    .expect("parse")
    .config;
    let d = app(&p, cfg).defaults();
    assert!(d.bots && d.generated);
    assert_eq!(
        d.exclude,
        ["*.lock", "docs/**"],
        "only this repository's override joins"
    );
    assert_eq!(d.period, Some(gitterrace_index::Period::Quarter));
}

#[tokio::test]
async fn insights_defaults_reach_the_pages() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let cfg = with(|c| {
        c.insights.bots = true;
        c.insights.generated = true;
        c.insights.exclude = vec!["small.txt".into()];
    });
    let r = router(&p, cfg);
    // The options row shows the state in force; wait for the index to be ready first.
    let mut body = String::new();
    for _ in 0..100 {
        body = get(&r, "/stats/frequency").await.1;
        if body.contains("Generated files") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(body.contains("Bots"), "{body}");
    let on = |label: &str, state: &str| {
        let at = body.find(label).expect(label);
        body[at..]
            .chars()
            .take(400)
            .collect::<String>()
            .contains(state)
    };
    assert!(on("Generated files", "counted") && on("Bots", "counted"));
    let (_, plain) = get(&router(&p, Config::default()), "/stats/frequency").await;
    assert!(
        !plain.contains("Exclude"),
        "the URL's own exclude row stays off"
    );
}

#[test]
fn multi_depth_and_skip_dirs_change_discovery() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path();
    repo(&root.join("a"), "one");
    repo(&root.join("a/b"), "two");
    repo(&root.join("target"), "three");
    let find = |depth, skip: &[&str]| {
        let skip: Vec<String> = skip.iter().map(|s| (*s).to_owned()).collect();
        let mut names: Vec<String> = crate::discover_repos(&[root.to_path_buf()], depth, &skip)
            .iter()
            .map(|p| p.strip_prefix(root).expect("under").display().to_string())
            .collect();
        names.sort();
        names
    };
    assert_eq!(find(2, &["target"]), ["a/one"]);
    assert_eq!(find(3, &["target"]), ["a/b/two", "a/one"]);
    assert_eq!(find(2, &[]), ["a/one", "target/three"]);
}

#[tokio::test]
async fn index_build_on_start_waits_for_demand_when_off() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let mut cfg = with(|c| c.index.build_on_start = false);
    cfg.watch.interval_ms = 250;
    let a = app(&p, cfg);
    tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&a)));
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(!a.ready(), "nothing builds until a page needs the index");
    let head = a.repo.resolve("HEAD").expect("head");
    assert!(
        a.records(&head).expect("records").is_none(),
        "asking is the demand"
    );
    for _ in 0..100 {
        if a.ready() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(a.ready(), "the demand starts the build");

    let on = app(&p, Config::default());
    tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&on)));
    for _ in 0..100 {
        if on.ready() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(on.ready(), "the default builds at once");
}

#[tokio::test]
async fn resources_blocking_jobs_bounds_concurrent_work() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let peak = |jobs: usize| {
        let a = app(&p, with(|c| c.resources.blocking_jobs = jobs));
        async move {
            let running = Arc::new(AtomicUsize::new(0));
            let max = Arc::new(AtomicUsize::new(0));
            let tasks: Vec<_> = (0..4)
                .map(|_| {
                    let (a, running, max) =
                        (Arc::clone(&a), Arc::clone(&running), Arc::clone(&max));
                    tokio::spawn(async move {
                        crate::app::blocking(&a, move |_| {
                            let n = running.fetch_add(1, Ordering::SeqCst) + 1;
                            max.fetch_max(n, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(80));
                            running.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        })
                        .await
                        .expect("job");
                    })
                })
                .collect();
            for t in tasks {
                t.await.expect("task");
            }
            max.load(Ordering::SeqCst)
        }
    };
    assert_eq!(peak(1).await, 1);
    assert!(peak(4).await > 1, "more slots run side by side");
}

#[test]
fn resources_cache_mb_sizes_the_code_cache() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    assert_eq!(
        app(&p, with(|c| c.resources.cache_mb = 8)).code_cache_bytes(),
        4 << 20
    );
    assert_eq!(
        app(&p, with(|c| c.resources.cache_mb = 64)).code_cache_bytes(),
        32 << 20
    );
    // A full cache drops what it held; a roomy one keeps both.
    let text = "fn a() {}\n".repeat(200);
    let cached = |name: &str| crate::views::code_is_cached(name, &text);
    crate::views::code_html(1, "wiring_tiny_a.rs", &text);
    crate::views::code_html(1, "wiring_tiny_b.rs", &text);
    assert!(!cached("wiring_tiny_a.rs") && cached("wiring_tiny_b.rs"));
    crate::views::code_html(1 << 24, "wiring_roomy_a.rs", &text);
    crate::views::code_html(1 << 24, "wiring_roomy_b.rs", &text);
    assert!(cached("wiring_roomy_a.rs") && cached("wiring_roomy_b.rs"));
}

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

async fn open_events(r: &Router, uri: &str) -> axum::response::Response {
    r.clone()
        .oneshot(
            Request::get(uri)
                .header("host", "localhost")
                .body(Body::empty())
                .expect("req"),
        )
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
            c.server.max_streams = 2;
            c.watch.interval_ms = 250;
        }),
    );
    let first = open_events(&server, "/events").await;
    assert_eq!(first.status(), 200);
    assert_eq!(first.headers()["content-type"], "text/event-stream");
    let mut first = first.into_body();
    let second = open_events(&server, "/events").await;
    assert_eq!(second.status(), 200);
    let third = open_events(&server, "/events").await;
    assert_eq!(
        third.status(),
        503,
        "server.max_streams = 2 refuses the third"
    );
    drop(second);
    drop(third);
    // Let the watcher take its first reading before the change.
    tokio::time::sleep(Duration::from_millis(600)).await;
    std::fs::write(work.join("small.txt"), "needle three\n").expect("write");
    git(&work, &["commit", "-qam", "fix: third"]);
    let got = read_until(&mut first, "event: change", Duration::from_secs(5)).await;
    let got = got.expect("a change event");
    assert!(got.contains("\"refs\""), "a commit moves the refs: {got}");
    // The slot of a closed stream is free again.
    assert_eq!(open_events(&server, "/events").await.status(), 200);
}

#[tokio::test]
async fn a_stream_that_missed_a_change_hears_it_on_connect() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let r = router(&p, with(|c| c.watch.interval_ms = 250));
    tokio::time::sleep(Duration::from_millis(600)).await;
    let mut body = open_events(&r, "/events?v=1").await.into_body();
    let got = read_until(&mut body, "event: change", Duration::from_secs(2)).await;
    assert!(
        got.is_some_and(|g| g.contains("\"index\"")),
        "a stale version hears everything"
    );
}

#[tokio::test]
async fn pages_carry_their_live_mode() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let r = router(&p, Config::default());
    let attr = |body: &str| {
        body.split("data-live=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .map(str::to_owned)
    };
    let (_, wt) = get(&r, "/worktree").await;
    assert_eq!(attr(&wt).as_deref(), Some("refresh"));
    assert!(wt.contains("/static/live.js"));
    let (_, st) = get(&r, "/stashes").await;
    assert_eq!(attr(&st).as_deref(), Some("refresh"));
    let (_, wts) = get(&r, "/worktrees").await;
    assert_eq!(attr(&wts).as_deref(), Some("refresh"));
    for notice in ["/log/HEAD", "/refs", "/stats", "/"] {
        assert_eq!(
            attr(&get(&r, notice).await.1).as_deref(),
            Some("notice"),
            "{notice}"
        );
    }
    let id = get(&r, "/log/HEAD").await.1;
    let id = id
        .split("/commit/")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("id")
        .to_owned();
    let (_, commit) = get(&r, &format!("/commit/{id}")).await;
    assert_eq!(attr(&commit), None, "a pinned commit never listens");
    assert!(!commit.contains("/static/live.js"));
    let off = router(&p, with(|c| c.watch.enabled = false));
    let (_, page) = get(&off, "/worktree").await;
    assert_eq!(attr(&page), None, "watch.enabled = false opens no streams");
}

#[tokio::test]
async fn watch_interval_sets_how_fast_a_change_is_seen() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    for (ms, fast) in [(250, true), (3000, false)] {
        let a = app(&p, with(|c| c.watch.interval_ms = ms));
        tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&a)));
        let mut rx = a.changes.subscribe();
        tokio::time::sleep(Duration::from_millis(400)).await;
        std::fs::write(p.join("small.txt"), format!("needle {ms}\n")).expect("write");
        git(&p, &["commit", "-qam", &format!("chore: {ms}")]);
        let got = tokio::time::timeout(Duration::from_millis(1200), rx.recv()).await;
        assert_eq!(got.is_ok(), fast, "interval {ms} ms");
    }
}

#[tokio::test]
async fn watch_off_runs_no_timer() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let a = app(&p, with(|c| c.watch.enabled = false));
    tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&a)));
    let mut rx = a.changes.subscribe();
    tokio::time::sleep(Duration::from_millis(300)).await;
    git(&p, &["commit", "-q", "--allow-empty", "-m", "chore: empty"]);
    let got = tokio::time::timeout(Duration::from_millis(1500), rx.recv()).await;
    assert!(got.is_err(), "no change is announced with the watcher off");
}

#[tokio::test]
async fn working_tree_is_scanned_only_for_a_subscriber() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let a = app(&p, with(|c| c.watch.interval_ms = 250));
    tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&a)));
    let mut rx = a.changes.subscribe();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let kinds = |c: Arc<crate::live::Change>| c.kinds.clone();
    std::fs::write(p.join("small.txt"), "edited without a subscriber\n").expect("write");
    // The cheap dirty check may announce the flip, but never a full worktree scan.
    while let Ok(Ok(c)) = tokio::time::timeout(Duration::from_millis(900), rx.recv()).await {
        assert!(
            !kinds(c).contains(&"worktree"),
            "no worktree scan without a subscriber"
        );
    }

    let _wt = crate::live::WorktreeWatch::new(&a);
    tokio::time::sleep(Duration::from_millis(600)).await;
    while rx.try_recv().is_ok() {}
    std::fs::write(p.join("small.txt"), "edited with a subscriber\n").expect("write");
    let got = tokio::time::timeout(Duration::from_millis(1500), rx.recv())
        .await
        .expect("event")
        .expect("open");
    assert!(kinds(got).contains(&"worktree"), "an edit is seen");
    std::fs::write(p.join("brand-new.txt"), "x\n").expect("write");
    let got = tokio::time::timeout(Duration::from_millis(1500), rx.recv())
        .await
        .expect("event")
        .expect("open");
    assert!(
        kinds(got).contains(&"worktree"),
        "an untracked file is seen"
    );
    let quiet = tokio::time::timeout(Duration::from_millis(900), rx.recv()).await;
    assert!(quiet.is_err(), "an untouched tree sends nothing");
}

#[tokio::test]
async fn dirty_chip_follows_on_pages_without_the_worktree() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let a = app(&p, with(|c| c.watch.interval_ms = 250));
    tokio::spawn(crate::app::keep_index_fresh(Arc::clone(&a)));
    // A listener, but no page that shows the working tree.
    let mut rx = a.changes.subscribe();
    tokio::time::sleep(Duration::from_millis(600)).await;
    while rx.try_recv().is_ok() {}
    let mut next = async |want: bool| {
        loop {
            let c = tokio::time::timeout(Duration::from_millis(2000), rx.recv())
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
        "version = 1\n[[action]]\nid = \"x\"\nlabel = \"X\"\nkinds = [\"page\"]\ncommand = [\"true\"]\nenv = [\"\"]\n",
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

/// Every `/static/...` file a template asks for is served. A script added to a template but
/// not to the asset table 404s, and the browser then refuses it for its empty MIME type.
#[tokio::test]
async fn every_static_file_a_template_loads_is_served() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let r = router(&p, Config::default());
    let templates =
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/templates")).expect("templates");
    let mut wanted = std::collections::BTreeSet::new();
    for entry in templates {
        let text = std::fs::read_to_string(entry.expect("entry").path()).expect("read");
        for (at, _) in text.match_indices("/static/") {
            let rest = &text[at..];
            let end = rest.find(['"', '\'', ' ', '>', '?']).unwrap_or(rest.len());
            wanted.insert(rest[..end].to_owned());
        }
    }
    assert!(wanted.len() >= 5, "found only {wanted:?}");
    for path in wanted {
        let (code, _) = get(&r, &path).await;
        assert_eq!(code, 200, "{path} is linked from a template but not served");
    }
}
