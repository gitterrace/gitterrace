//! Every setting the server honours has a test here that changes it and sees behaviour change.

use std::{path::Path, sync::Arc, time::Duration};

use axum::{Router, body::Body, http::Request};
use gitterrace_config::Config;
use gitterrace_testkit::{Fixture, git, page as get};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::app::{App, Env};

/// A repository with a small file, a 3 KiB file and two commits.
pub(crate) fn repo(root: &Path, name: &str) -> std::path::PathBuf {
    Fixture::init(&root.join(name))
        .write("small.txt", "needle one\n")
        .write("mid.txt", &"needle\n".repeat(450))
        .write("yarn.lock", "lock\n")
        .commit("feat: first")
        .write("small.txt", "needle two\n")
        .commit("fix: second")
        .path()
        .to_owned()
}

pub(crate) fn with(f: impl FnOnce(&mut Config)) -> Config {
    let mut c = Config::default();
    f(&mut c);
    c.validate().expect("valid");
    c
}

pub(crate) fn router(p: &Path, cfg: Config) -> Router {
    crate::app_router_with(p, cfg).expect("router")
}

pub(crate) fn app(p: &Path, cfg: Config) -> Arc<App> {
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
    assert!(body.contains("File too large to display"), "text_kb = 1 refuses it");
    let (_, body) = get(&router(&p, with(|c| c.limits.text_kb = 1)), "/blame/HEAD/mid.txt").await;
    assert!(body.contains("too large"));

    let big = "x\n".repeat(600 * 1024);
    std::fs::write(p.join("big.txt"), &big).expect("write");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-qm", "chore: big"]);
    let (s, _) = get(&router(&p, Config::default()), "/raw/HEAD/big.txt").await;
    assert_eq!(s, 200);
    let tiny = with(|c| c.limits.raw_mb = 1);
    let (s, body) = get(&router(&p, tiny), "/raw/HEAD/big.txt").await;
    assert_eq!(s, 404, "1.2 MB is over 1 MiB, and 413 would name the request");
    assert!(body.contains("limits.raw_mb"), "{body}");
}

#[tokio::test]
async fn raw_etag_is_the_blob_id_and_honours_if_none_match() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path().to_path_buf();
    git(&p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "hello\n").expect("write");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-qm", "feat: a"]);
    let r = router(&p, Config::default());
    let raw = format!("{}/raw/HEAD/a.txt", gitterrace_testkit::base(&r).await);
    let send = |inm: Option<&'static str>| {
        let (r, raw) = (r.clone(), raw.clone());
        async move {
            let mut req = Request::get(raw).header("host", "localhost");
            if let Some(v) = inm {
                req = req.header("if-none-match", v);
            }
            let res = r.oneshot(req.body(Body::empty()).expect("req")).await;
            res.expect("res")
        }
    };
    let first = send(None).await;
    assert_eq!(first.status().as_u16(), 200);
    let tag = first.headers().get("etag").expect("etag").to_str().expect("ascii").to_owned();
    // 40 hex chars of the blob id, quoted.
    assert_eq!(tag.len(), 42, "{tag}");
    let tag: &'static str = Box::leak(tag.into_boxed_str());
    let hit = send(Some(tag)).await;
    assert_eq!(hit.status().as_u16(), 304);
    assert_eq!(hit.headers().get("etag").expect("etag"), tag);
    let body = hit.into_body().collect().await.expect("body").to_bytes();
    assert!(body.is_empty());
    assert_eq!(send(Some("\"nope\"")).await.status().as_u16(), 200);
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

/// The first commit id on `/log/HEAD`, the newest.
async fn newest_commit(r: &Router) -> String {
    let (_, head) = get(r, "/log/HEAD").await;
    head.split("/commit/")
        .nth(1)
        .and_then(|s| s.split(['"', '#']).next())
        .expect("a commit link")
        .to_owned()
}

/// A lockfile and a `linguist-generated` file fold under a Generated tag, a normal file stays
/// open, and `-linguist-generated` unfolds a lockfile. Search lists generated hits last.
#[tokio::test]
async fn generated_files_fold_in_the_diff_and_sort_last_in_search() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = Fixture::init(&dir.path().join("g"))
        .write(".gitattributes", "gen.rs linguist-generated\nkeep/yarn.lock -linguist-generated\n")
        .write("Cargo.lock", "needle\n")
        .write("gen.rs", "needle\n")
        .write("main.rs", "needle\n")
        .write("keep/yarn.lock", "needle\n")
        .commit("feat: all")
        .path()
        .to_owned();
    let r = router(&p, Config::default());
    let id = newest_commit(&r).await;
    let (_, page) = get(&r, &format!("/commit/{id}")).await;
    let section = |path: &str| {
        let at = page.find(&format!("data-path=\"{path}\"")).expect("section");
        let rest = &page[at..];
        rest[..rest.find("</section>").expect("end")].to_owned()
    };
    for folded in ["Cargo.lock", "gen.rs"] {
        let s = section(folded);
        assert!(s.contains(">Generated</span>") && s.contains("data-load-diff"), "{s}");
    }
    for open in ["main.rs", "keep/yarn.lock"] {
        let s = section(open);
        assert!(!s.contains("Generated") && !s.contains("data-load-diff"), "{s}");
    }
    let (_, hits) = get(&r, "/search?q=needle").await;
    let pos = |path: &str| hits.find(&format!("data-path=\"{path}\"")).expect(path);
    assert!(pos("keep/yarn.lock") < pos("Cargo.lock"));
    assert!(pos("main.rs") < pos("Cargo.lock") && pos("main.rs") < pos("gen.rs"));
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
    assert!(folded.contains("data-load-diff"), "two changed lines fold at 1");
}

#[tokio::test]
async fn display_theme_and_dates_change_the_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, body) = get(&router(&p, Config::default()), "/log/HEAD").await;
    assert!(!body.contains("data-theme") && body.contains("just now"));
    let dark = with(|c| {
        c.display.theme = gitterrace_types::ColorScheme::Dark;
        c.display.dates = gitterrace_types::DateStyle::Absolute;
    });
    let (_, body) = get(&router(&p, dark), "/log/HEAD").await;
    assert!(body.contains("<html lang=\"en\" data-theme=\"dark\">"));
    assert!(!body.contains("just now") && !body.contains("ago"), "dates are absolute");
}

/// `display.sidebar = []` drops the sidebar and lets the files take the full width.
#[tokio::test]
async fn an_empty_sidebar_list_hides_the_sidebar() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, body) = get(&router(&p, with(|c| c.display.sidebar.clear())), "/").await;
    assert!(!body.contains("<aside>"), "{body}");
    assert!(body.contains("<div class=\"cols wide\">"), "{body}");
    // The sidebar holds insights, so it shows once the index is ready, never as a progress box.
    let r = router(&p, Config::default());
    let body = gitterrace_testkit::wait::page_until(&r, "/", |b| b.contains("<aside>")).await;
    assert!(body.contains("<div class=\"cols\">"), "{body}");
}

/// A count sits next to the noun that agrees with it: one commit by one author reads singular.
#[tokio::test]
async fn overview_counts_agree_with_their_noun() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = Fixture::init(&dir.path().join("one"))
        .write("a.txt", "x\n")
        .commit("feat: only")
        .path()
        .to_owned();
    let r = router(&p, Config::default());
    let body =
        gitterrace_testkit::wait::page_until(&r, "/", |b| b.contains("class=\"facts\"")).await;
    assert!(body.contains("class=\"facts\""), "{body}");
    for (want, wrong) in [
        ("</b> commit<", "</b> commits<"),
        ("</b> author<", "</b> authors<"),
        ("</b> file touched<", "</b> files touched<"),
    ] {
        assert!(body.contains(want), "missing {want}: {body}");
        assert!(!body.contains(wrong), "found {wrong}: {body}");
    }
}

#[test]
fn insights_defaults_follow_the_config_and_the_repo_override() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "alpha");
    let none = app(&p, Config::default()).defaults();
    assert!(!none.bots && !none.generated && none.exclude.is_empty() && none.period.is_none());

    let cfg = gitterrace_config::parse(
        "[insights]\ninclude_bots = true\ninclude_generated = true\nperiod = \"3m\"\nexclude = \
        [\"*.lock\"]\n\n[repo.alpha]\nexclude = [\"docs/**\"]\n[repo.beta]\nexclude = [\"nope\"]\n",
    )
    .expect("parse")
    .config;
    let d = app(&p, cfg).defaults();
    assert!(d.bots && d.generated);
    assert_eq!(d.exclude, ["*.lock", "docs/**"], "only this repository's override joins");
    assert_eq!(d.period, Some(gitterrace_index::Period::Quarter));
}

#[tokio::test]
async fn insights_defaults_reach_the_pages() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let cfg = with(|c| {
        c.insights.include_bots = true;
        c.insights.include_generated = true;
        c.insights.exclude = vec!["small.txt".into()];
    });
    let r = router(&p, cfg);
    // The options row shows the state in force; wait for the index to be ready first.
    let body = gitterrace_testkit::wait::page_until(&r, "/stats/hotspots", |b| {
        b.contains("Generated files")
    })
    .await;
    assert!(body.contains("Bots"), "{body}");
    let on = |label: &str, state: &str| {
        let at = body.find(label).expect(label);
        body[at..].chars().take(400).collect::<String>().contains(state)
    };
    assert!(on("Generated files", "counted") && on("Bots", "counted"));
    let (_, plain) = get(&router(&p, Config::default()), "/stats/hotspots").await;
    assert!(!plain.contains("Exclude"), "the URL's own exclude row stays off");
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
            // Joined with `/` on every system: Windows displays its own separator.
            .map(|p| {
                let rel = p.strip_prefix(root).expect("under");
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
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
    let cfg = with(|c| c.index.build_on_start = false);
    let a = app(&p, cfg);
    tokio::spawn(crate::app::keep_index_fresh_supervised(Arc::clone(&a)));
    // A quiet window: nothing may start the build without a demand.
    gitterrace_testkit::wait::pause(Duration::from_millis(700)).await;
    assert!(!a.ready(), "nothing builds until a page needs the index");
    let head = a.repo.resolve("HEAD").expect("head");
    assert!(a.source(&head).expect("source").is_none(), "asking is the demand");
    gitterrace_testkit::wait::until("the demand starts the build", || async { a.ready() }).await;

    let on = app(&p, Config::default());
    tokio::spawn(crate::app::keep_index_fresh_supervised(Arc::clone(&on)));
    gitterrace_testkit::wait::until("the default builds at once", || async { on.ready() }).await;
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
                            gitterrace_testkit::wait::pause_blocking(Duration::from_millis(80));
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

#[tokio::test]
async fn a_cancelled_request_keeps_its_job_slot_until_the_work_ends() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let a = app(&p, with(|c| c.resources.blocking_jobs = 2));
    let done = Arc::new(AtomicBool::new(false));
    let d = Arc::clone(&done);
    let job = crate::app::blocking(&a, move |_| {
        gitterrace_testkit::wait::pause_blocking(Duration::from_millis(300));
        d.store(true, Ordering::SeqCst);
        Ok(())
    });
    // Dropped like a timed-out public request: the job itself runs on.
    let cut = tokio::time::timeout(Duration::from_millis(1), job).await;
    assert!(cut.is_err(), "the job outlives the timeout");
    assert!(!done.load(Ordering::SeqCst));
    assert_eq!(a.env.free_jobs(), 1, "the running job still holds its slot");
    gitterrace_testkit::wait::until("the job ends", || async { done.load(Ordering::SeqCst) }).await;
    gitterrace_testkit::wait::until("its slot comes back", || async { a.env.free_jobs() == 2 })
        .await;
}

#[test]
fn resources_cache_mb_sizes_the_code_cache() {
    // A memo of its own: the process-wide cache is shared with every test that renders a
    // blob, and one of those flushing it between two lines here made this test flaky.
    use crate::views::{CodeMemo, code_memo_has as has, code_memo_put as put};
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    assert_eq!(app(&p, with(|c| c.resources.cache_mb = 8)).code_cache_bytes(), 3 << 19);
    assert_eq!(app(&p, with(|c| c.resources.cache_mb = 64)).code_cache_bytes(), 12 << 20);
    // A roomy cache keeps both; one file over an eighth of the cache is not kept at all.
    let text = "fn a() {}\n".repeat(200);
    let mut m = CodeMemo::default();
    let one = put(&mut m, 1 << 24, "size.rs", &text);
    let mut m = CodeMemo::default();
    put(&mut m, one * 8, "tiny_a.rs", &text);
    put(&mut m, one * 8, "tiny_b.rs", &text);
    assert!(has(&m, "tiny_a.rs", &text) && has(&m, "tiny_b.rs", &text));
    put(&mut m, one * 8 - 1, "over_eighth.rs", &text);
    assert!(!has(&m, "over_eighth.rs", &text));
    let mut m = CodeMemo::default();
    put(&mut m, 1 << 24, "roomy_a.rs", &text);
    put(&mut m, 1 << 24, "roomy_b.rs", &text);
    assert!(has(&m, "roomy_a.rs", &text) && has(&m, "roomy_b.rs", &text));
}

#[tokio::test]
async fn raw_budget_blocks_a_third_concurrent_reader_until_a_body_is_sent() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let a = app(
        &p,
        with(|c| {
            c.limits.raw_mb = 1;
            c.limits.raw_concurrent_mb = 2;
        }),
    );
    let r = crate::routes::router(Arc::clone(&a));
    let send = |r: Router| async move {
        let req = Request::get("/raw/HEAD/mid.txt").body(Body::empty()).expect("req");
        r.oneshot(req).await.expect("response")
    };
    // Unread bodies: each still holds its share of the budget.
    let first = send(r.clone()).await;
    let second = send(r.clone()).await;
    assert_eq!(first.status(), 200);
    assert_eq!(a.env.raw_budget.available_permits(), 0, "two readers fill it");
    let third = tokio::spawn(send(r.clone()));
    gitterrace_testkit::wait::pause(Duration::from_millis(100)).await;
    assert!(!third.is_finished(), "the third waits before reading");
    let bytes = first.into_body().collect().await.expect("body").to_bytes();
    assert_eq!(bytes.len(), 450 * 7, "the body is the whole file");
    let third = tokio::time::timeout(Duration::from_secs(5), third)
        .await
        .expect("served once a body is sent")
        .expect("task");
    assert_eq!(third.status(), 200);
    drop((second, third));
    assert_eq!(a.env.raw_budget.available_permits(), 2, "every share comes back");
}

#[test]
fn raw_mb_defaults_to_ten_and_refuses_one_byte_over() {
    let mb = Config::default().limits.raw_mb;
    assert_eq!(mb, 10);
    assert!(!crate::routes::raw_refused(mb << 20, mb));
    assert!(crate::routes::raw_refused((mb << 20) + 1, mb));
    let zero = with(|_| {});
    let mut bad = zero;
    bad.limits.raw_concurrent_mb = 0;
    assert!(bad.validate().is_err(), "a zero budget would serve nothing");
}

/// Guard class: background work that fills caches runs under the warming guard. A root walk
/// held in flight when the idle release runs defers it, so nothing refills the caches after
/// it, and the release lands once the walk ends. No timing: the hold is a latch and the walk's
/// handle is awaited.
#[tokio::test]
async fn an_idle_release_waits_for_an_in_flight_related_warm() {
    let (dir, app, head) = crate::tests::one_commit_app();
    app.env.related.add(&app.key, dir.path().to_owned());
    drop(app.source(&head).expect("source"));
    assert!(app.source_held(), "a read filled the source");
    crate::related::hold::set(dir.path(), true);
    let walk = app.warm_related();
    assert!(!app.release_idle(Duration::ZERO), "released under a warm in flight");
    assert!(app.source_held(), "the caches were cut while the warm ran");
    crate::related::hold::set(dir.path(), false);
    walk.await.expect("walk");
    assert!(app.release_idle(Duration::ZERO), "the release did not follow the warm");
    assert!(!app.source_held(), "a cache survived the release");
    assert!(!app.release_idle(Duration::ZERO), "nothing left to release");
}

/// Guard class: every `Slots` (jobs, live streams, the raw budget) honours a cut made while
/// slots are busy. Two of three held, cut to one: the cut is owed, and once both come back one
/// slot exists, not three.
#[tokio::test]
async fn a_cut_made_while_slots_are_busy_lands_when_they_return() {
    let slots = crate::app::Slots::new(3);
    let (a, b) = (slots.take().await.expect("slot"), slots.take().await.expect("slot"));
    slots.resize(1);
    drop((a, b));
    assert_eq!(slots.available_permits(), 1, "the old cap came back");
    let held = slots.try_take().expect("the one slot in force");
    assert!(slots.try_take().is_none(), "a second slot exists past the cut");
    drop(held);
    assert_eq!(slots.available_permits(), 1);
}

/// A raise after a cut pays the debt first, then adds only what is left, and a raise past the
/// old cap grows it.
#[tokio::test]
async fn a_raise_after_a_cut_pays_the_debt_first() {
    let slots = crate::app::Slots::new(3);
    let held = [
        slots.take().await.expect("slot"),
        slots.take().await.expect("slot"),
        slots.take().await.expect("slot"),
    ];
    slots.resize(1);
    slots.resize(2);
    drop(held);
    assert_eq!(slots.available_permits(), 2, "the raise did not cancel the debt");
    slots.resize(4);
    assert_eq!(slots.available_permits(), 4);
    slots.resize(1);
    assert_eq!(slots.available_permits(), 1, "a free cut lands at once");
}
