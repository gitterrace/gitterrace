//! A selection narrows which commits count, never the days a period means, who counts as new,
//! or which head files a path covers. The API and the statistics pages read the same rule.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, oracle, page};

/// Bob: a feat on 2024-01-10, a fix on 2024-03-10. Ann: the newest commit, 2024-03-20, which
/// adds `docs/guide.md` (outside `src`).
fn repo(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("repo");
    let mut f = Fixture::init(&p);
    for (who, date, path, msg) in [
        (
            "Bob <bob@y.io>",
            "2024-01-10T12:00:00Z",
            "src/a.rs",
            "feat: a",
        ),
        (
            "Bob <bob@y.io>",
            "2024-03-10T12:00:00Z",
            "src/a.rs",
            "fix: a",
        ),
        (
            "Ann <ann@x.io>",
            "2024-03-20T12:00:00Z",
            "docs/guide.md",
            "docs: guide",
        ),
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
    v.as_array()
        .expect("buckets")
        .iter()
        .map(|b| b["key"].clone())
        .collect()
}

/// "Last month" ends on the repository's newest day, with or without a chip.
#[test]
fn a_chip_keeps_the_period_anchor() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let router = gitterrace_web::app_router(repo(dir.path())).expect("router");
        oracle::ready(&router).await;
        let api = "/api/v1/insights";
        let all = json(&router, &format!("{api}/summary?period=1m")).await;
        let bob = json(
            &router,
            &format!("{api}/summary?period=1m&author=bob%40y.io"),
        )
        .await;
        assert_eq!(
            (&bob["from"], &bob["to"]),
            (&all["from"], &all["to"]),
            "{bob}"
        );
        let fa = json(&router, &format!("{api}/frequency?period=1m")).await;
        let fb = json(&router, &format!("{api}/frequency?period=1m&type=fix")).await;
        assert_eq!(keys(&fb), keys(&fa), "frequency buckets with a chip");
    });
}

/// Bob's first commit ever is before the month, so his fix in it does not make him new,
/// even when the type chip hides that first commit.
#[test]
fn new_is_judged_on_the_whole_history() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let router = gitterrace_web::app_router(repo(dir.path())).expect("router");
        oracle::ready(&router).await;
        let c = json(&router, "/api/v1/insights/contributors?period=1m&type=fix").await;
        let bob = c["contributors"]
            .as_array()
            .expect("people")
            .iter()
            .find(|p| p["name"] == "Bob")
            .expect("Bob")
            .clone();
        assert_eq!(bob["first_time"], false, "{bob}");
    });
}

/// Under `path=src`, `docs/guide.md` is not a line of the path, tracked or not.
#[test]
fn a_path_covers_only_its_head_files() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let router = gitterrace_web::app_router(repo(dir.path())).expect("router");
        oracle::ready(&router).await;
        let o = json(&router, "/api/v1/insights/ownership?period=all&path=src").await;
        assert_eq!(o["untracked_lines"], 0, "{o}");
    });
}
