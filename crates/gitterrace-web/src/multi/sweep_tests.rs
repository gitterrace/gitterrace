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
    let (s, page) = get(&router, "/r/alpha/").await;
    assert_eq!(s, 200, "{page}");
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
