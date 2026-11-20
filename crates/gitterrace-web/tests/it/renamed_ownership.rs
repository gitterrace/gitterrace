//! Ownership over all time on a renamed-and-reused path.
//!
//! `src/lib.rs` moves to `src/core.rs` and a new `src/lib.rs` starts over. The moved file keeps
//! the old history's authors (Bob and Jane); the new file has only the author of its own
//! commit (Jane). Reading the path x author aggregate for a path that took part in a rename
//! would credit the new file with the old file's authors. The golden for these cells had no
//! reference, so this pins the meaning. Each file is shown alone by excluding the other.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::matrix::Mailmap;
use gitterrace_testkit::{Fixture, oracle, page};

/// The owners of directory `src` in the ownership API, with the other file excluded.
async fn src_owners(router: &axum::Router, other: &str) -> Vec<String> {
    let uri = format!("/api/v1/insights/ownership?period=all&bots=0&exclude={other}");
    let (status, body) = page(router, &uri).await;
    assert_eq!(status, 200, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    let dirs = json["data"]["dirs"].as_array().expect("dirs");
    let src = dirs.iter().find(|d| d["path"] == "src").expect("src dir");
    assert_eq!(src["files"], 1, "{body}");
    src["owners"]
        .as_array()
        .expect("owners")
        .iter()
        .map(|o| o["name"].as_str().expect("name").to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reused_path_does_not_inherit_the_moved_file_history() {
    let dir = tempfile::tempdir().expect("tmp");
    let (repo, _) = oracle::matrix_repo(dir.path(), Mailmap::AsIs);
    let f = Fixture::init(&repo);
    f.git(&["mv", "src/lib.rs", "src/core.rs"]);
    let f = f
        .at("2024-03-09T11:00:00Z")
        .commit("refactor(core): move lib to core")
        .at("2024-03-09T12:00:00Z")
        .write("src/lib.rs", "pub mod core;\n")
        .commit("feat(core): a new lib");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    oracle::ready(&router).await;

    let new_lib = src_owners(&router, "src/core.rs").await;
    assert_eq!(new_lib, ["Jane Doe"], "the new src/lib.rs has its own author only");
    let core = src_owners(&router, "src/lib.rs").await;
    assert!(
        core.contains(&"Bob".to_owned()) && core.contains(&"Jane Doe".to_owned()),
        "src/core.rs keeps the old history's authors: {core:?}"
    );

    let (status, html) = page(&router, "/stats/ownership?period=all&exclude=src/core.rs").await;
    assert_eq!(status, 200);
    let row = html
        .split("<tr>")
        .find(|r| r.contains("<span class=\"seg\">src</span></a>"))
        .expect("src row");
    assert!(row.contains("Jane Doe") && !row.contains("Bob"), "{row}");
}

/// Lines and owners of directory `src` in the ownership API for `query`.
async fn src_dir(router: &axum::Router, query: &str) -> serde_json::Value {
    let uri = format!("/api/v1/insights/ownership?period=all&bots=0&{query}");
    let (status, body) = page(router, &uri).await;
    assert_eq!(status, 200, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    let dirs = json["data"]["dirs"].as_array().expect("dirs").clone();
    dirs.into_iter().find(|d| d["path"] == "src").unwrap_or(serde_json::Value::Null)
}

/// An author selection must not change how a file's history is followed. Bob wrote
/// `src/old.rs`, Jane renamed it to `src/new.rs`. With `author=bob` the renamed file is still
/// Bob's: follow over the whole history, then count his commits.
#[tokio::test(flavor = "multi_thread")]
async fn an_author_selection_follows_renames_made_by_others() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path().join("repo");
    let f = Fixture::init(&p).at("2024-03-01T12:00:00Z").write("src/old.rs", "a\nb\nc\nd\n");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-q", "--author", "Bob <bob@y.io>", "-m", "feat: old"]);
    let f = f.at("2024-03-02T12:00:00Z");
    f.git(&["mv", "src/old.rs", "src/new.rs"]);
    f.git(&["commit", "-q", "--author", "Jane <jane@x.io>", "-m", "refactor: rename"]);
    let router = gitterrace_web::app_router(f.path()).expect("router");
    oracle::ready(&router).await;

    let all = src_dir(&router, "").await;
    let bob = src_dir(&router, "q=author%3Abob%40y.io").await;
    assert_eq!(all["owners"][0]["name"], "Bob", "{all}");
    assert_eq!(bob["lines"], all["lines"], "author=bob: {bob}");
    assert_eq!(bob["owners"][0]["name"], "Bob", "{bob}");
}
