//! A selection narrows which commits count, never the days a period means, who counts as new,
//! or which head files a path covers. The API and the statistics pages read the same rule.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, page};

/// Bob: a feat on 2024-01-10, a fix on 2024-03-10. Ann: the newest commit, 2024-03-20, which
/// adds `docs/guide.md` (outside `src`).
fn repo(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("repo");
    let mut f = Fixture::init(&p);
    for (who, date, path, msg) in [
        ("Bob <bob@y.io>", "2024-01-10T12:00:00Z", "src/a.rs", "feat: a"),
        ("Bob <bob@y.io>", "2024-03-10T12:00:00Z", "src/a.rs", "fix: a"),
        ("Ann <ann@x.io>", "2024-03-20T12:00:00Z", "docs/guide.md", "docs: guide"),
    ] {
        f = f.at(date).write(path, &format!("{msg}\nmore\n"));
        f.git(&["add", "-A"]);
        f.git(&["commit", "-q", "--author", who, "-m", msg]);
    }
    p
}

async fn json(router: &axum::Router, uri: &str) -> serde_json::Value {
    let (status, body) = page(router, uri).await;
    assert_eq!(status, 200, "{uri}: {body}");
    serde_json::from_str::<serde_json::Value>(&body).expect("json")["data"].clone()
}

fn keys(v: &serde_json::Value) -> Vec<serde_json::Value> {
    v.as_array().expect("buckets").iter().map(|b| b["key"].clone()).collect()
}

/// "Last month" ends on the repository's newest day, with or without a chip.
#[tokio::test(flavor = "multi_thread")]
async fn a_chip_keeps_the_period_anchor() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = crate::common::ready_router(repo(dir.path())).await;
    let api = "/api/v1/insights";
    let all = json(&router, &format!("{api}/summary?period=1m")).await;
    let bob = json(&router, &format!("{api}/summary?q=author:bob%40y.io&period=1m")).await;
    assert_eq!((&bob["from"], &bob["to"]), (&all["from"], &all["to"]), "{bob}");
    let fa = json(&router, &format!("{api}/frequency?period=1m")).await;
    let fb = json(&router, &format!("{api}/frequency?q=type:fix&period=1m")).await;
    assert_eq!(keys(&fb), keys(&fa), "frequency buckets with a chip");
}

/// Bob's first commit ever is before the month, so his fix in it does not make him new,
/// even when the type chip hides that first commit.
#[tokio::test(flavor = "multi_thread")]
async fn new_is_judged_on_the_whole_history() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = crate::common::ready_router(repo(dir.path())).await;
    let c = json(&router, "/api/v1/insights/contributors?q=type:fix&period=1m").await;
    let bob = c["contributors"]
        .as_array()
        .expect("people")
        .iter()
        .find(|p| p["name"] == "Bob")
        .expect("Bob")
        .clone();
    assert_eq!(bob["first_time"], false, "{bob}");
}

/// Under `path=src`, `docs/guide.md` is not a line of the path, tracked or not.
#[tokio::test(flavor = "multi_thread")]
async fn a_path_covers_only_its_head_files() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = crate::common::ready_router(repo(dir.path())).await;
    let o = json(&router, "/api/v1/insights/ownership?q=path:src&period=all").await;
    assert_eq!(o["untracked_lines"], 0, "{o}");
}

/// The hotspots page, patterns included, is empty when the period holds no selected commit,
/// judged on the repository's newest day: Bob's last commit is ten days before Ann's, so his week
/// is empty.
#[tokio::test(flavor = "multi_thread")]
async fn patterns_empty_state_follows_the_period_anchor() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = crate::common::ready_router(repo(dir.path())).await;
    let (status, body) = page(&router, "/stats/hotspots?q=author:bob%40y.io&period=1w").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("No commits match these filters."), "{body}");
    let (_, all) = page(&router, "/stats/hotspots?q=author:bob%40y.io&period=all").await;
    assert!(!all.contains("No commits match these filters."), "{all}");
}
