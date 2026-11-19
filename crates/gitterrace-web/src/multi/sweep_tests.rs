use std::path::{Path, PathBuf};

use axum::Router;
use gitterrace_testkit::{Fixture, get, wait};

/// A searched folder `root` holding `alpha` and `beta`, served as `git terrace root` does.
fn served(root: &Path) -> Router {
    for name in ["alpha", "beta"] {
        repo(&root.join(name));
    }
    let mut cfg = gitterrace_config::Config::default();
    cfg.watch.rescan_secs = 5;
    let mut env = crate::app::Env::new(cfg);
    let search = vec![root.to_path_buf()];
    let paths = crate::discover_repos(&search, 2, &[]);
    env.search = crate::search_roots(&search, 2, &[]);
    assert_eq!(env.search, search, "the root is searched");
    crate::build_server(paths, None, None, &env, None, None)
        .expect("server")
        .0
}

fn repo(dir: &Path) -> PathBuf {
    Fixture::init(dir)
        .write("a.txt", "a\n")
        .commit("feat: first")
        .path()
        .to_path_buf()
}

async fn listed(router: &Router) -> Vec<String> {
    let (_, list) = get(router, "/api/repos").await;
    serde_json::from_str::<Vec<serde_json::Value>>(&list)
        .expect("json")
        .iter()
        .filter_map(|i| i["name"].as_str().map(str::to_owned))
        .collect()
}

/// The table lists exactly `want` (sorted), and the home and the gone repository's page agree.
async fn awaits(router: &Router, want: &[&str], gone: Option<&str>) {
    wait::until(&format!("the home lists {want:?}"), || async move {
        let mut now = listed(router).await;
        now.sort();
        if now != want {
            return false;
        }
        let (home, _) = get(router, "/").await;
        let shown = if want.len() == 1 {
            home == 302
        } else {
            home == 200
        };
        match gone {
            Some(name) => shown && get(router, &format!("/r/{name}/")).await.0 == 404,
            None => shown,
        }
    })
    .await;
}

/// A repository folder deleted under the served root leaves the home and answers 404, with no
/// restart; made again in the same place, it comes back under its name.
#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_repository_folder_leaves_and_a_recreated_one_returns() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().canonicalize().expect("canon");
    let router = served(&root);
    awaits(&router, &["alpha", "beta"], None).await;
    let alpha = root.join("alpha");
    wait::until_blocking("removing alpha", || std::fs::remove_dir_all(&alpha).is_ok());
    awaits(&router, &["beta"], Some("alpha")).await;
    let (_, home) = get(&router, "/").await;
    assert!(!home.contains("/r/alpha/"), "{home}");
    repo(&root.join("alpha"));
    awaits(&router, &["alpha", "beta"], None).await;
    // A found repository answers 503 until its router is built.
    wait::until("alpha opens", || async {
        get(&router, "/r/alpha/").await.0 == 200
    })
    .await;
}

/// A folder that stays but whose `.git` is removed is no repository any more: it is dropped too.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_whose_git_directory_is_removed_is_dropped() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().canonicalize().expect("canon");
    let router = served(&root);
    awaits(&router, &["alpha", "beta"], None).await;
    // The index cache may still be writing inside `.git`: remove until nothing is left.
    let git = root.join("beta/.git");
    wait::until_blocking("removing .git", || std::fs::remove_dir_all(&git).is_ok());
    assert!(root.join("beta/a.txt").exists(), "the folder stays");
    awaits(&router, &["alpha"], Some("beta")).await;
}

/// The watch skips everything inside a repository but its `.git` entry, so an edit to a
/// tracked file, or the index cache writing, costs no sweep.
#[test]
fn the_watch_sees_repository_entries_but_not_their_contents() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().canonicalize().expect("canon");
    repo(&root.join("alpha"));
    let skip = super::skipper(vec![root.clone()], 2, vec!["target".to_owned()]);
    for seen in ["", "alpha", "alpha/.git", "group", "group/new"] {
        assert!(!skip(&root.join(seen), true), "{seen}");
    }
    for skipped in [
        "alpha/a.txt",
        "alpha/.git/HEAD",
        "alpha/src",
        "target",
        "a/b/c/d",
    ] {
        assert!(skip(&root.join(skipped), true), "{skipped}");
    }
    assert!(skip(Path::new("/elsewhere"), true));
}

/// Reads the home stream until it has carried `n` list notices in all, at most the shared
/// deadline, keeping what it read in `seen`.
async fn notices(events: &mut axum::body::Body, seen: &mut String, n: usize) {
    use http_body_util::BodyExt;
    let got = tokio::time::timeout(wait::DEADLINE, async {
        while seen.matches("\"repos\"").count() < n {
            match events.frame().await {
                Some(Ok(f)) => {
                    if let Some(d) = f.data_ref() {
                        seen.push_str(&String::from_utf8_lossy(d));
                    }
                }
                _ => return false,
            }
        }
        true
    })
    .await;
    assert_eq!(got, Ok(true), "{n} list notices on the home stream: {seen}");
}

/// Guard class: an open home follows the repository list, whatever changes it. The page says
/// it follows (`data-live`), and its stream hears the sweep add a folder made under the root
/// and drop it again once deleted. Red when the home has no stream, or the sweep announces
/// to the repositories' streams alone.
#[tokio::test(flavor = "multi_thread")]
async fn an_open_home_hears_the_sweep_add_and_drop_a_repository() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().canonicalize().expect("canon");
    let router = served(&root);
    awaits(&router, &["alpha", "beta"], None).await;
    let (_, home) = get(&router, "/").await;
    assert!(home.contains("data-live=\"home\""), "{home}");
    assert!(home.contains("/static/live.js"), "{home}");
    let res = router
        .clone()
        .oneshot(
            axum::http::Request::get("/events")
                .header("host", "localhost")
                .body(axum::body::Body::empty())
                .expect("req"),
        )
        .await
        .expect("events");
    assert_eq!(res.status(), 200);
    let mut events = res.into_body();
    let mut seen = String::new();
    repo(&root.join("gamma"));
    notices(&mut events, &mut seen, 1).await;
    awaits(&router, &["alpha", "beta", "gamma"], None).await;
    let gamma = root.join("gamma");
    wait::until_blocking("removing gamma", || std::fs::remove_dir_all(&gamma).is_ok());
    notices(&mut events, &mut seen, 2).await;
    awaits(&router, &["alpha", "beta"], Some("gamma")).await;
}

/// The home reads every repository on its own threads, so it takes a page job slot like any
/// page: with every slot held it waits, and it answers once one is free.
#[tokio::test(flavor = "multi_thread")]
async fn the_home_waits_for_a_page_job_slot() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().canonicalize().expect("canon");
    for name in ["alpha", "beta"] {
        repo(&root.join(name));
    }
    let mut cfg = gitterrace_config::Config::default();
    cfg.resources.blocking_jobs = 1;
    let env = crate::app::Env::new(cfg);
    let paths = crate::discover_repos(std::slice::from_ref(&root), 2, &[]);
    let router = crate::build_server(paths, None, None, &env, None, None)
        .expect("server")
        .0;
    let held = env.job_slot().await.expect("slot");
    let home = tokio::spawn({
        let router = router.clone();
        async move { get(&router, "/").await.0 }
    });
    wait::pause(std::time::Duration::from_millis(100)).await;
    assert!(!home.is_finished(), "the home waits for the held slot");
    drop(held);
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), home)
        .await
        .expect("served once the slot is free")
        .expect("task");
    assert_eq!(status, 200);
}
