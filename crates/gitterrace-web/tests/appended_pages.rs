//! Every insights page and API insight is byte for byte the same over a fresh build and over
//! the same history built by appends. Life ids (and string ids) may differ between the two
//! until the background merge, the pages never may. On the rename chain, whose dates are old,
//! so a period window that moves with the clock between the two captures cuts nothing.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::collections::BTreeMap;
use std::path::Path;

use gitterrace_testkit::{RenameChain, git, oracle, page};

fn targets() -> Vec<String> {
    let names: Vec<&str> = gitterrace_api::insight_list().map(|(n, _)| n).collect();
    oracle::targets(&gitterrace_web::WARM_PAGES, &names)
}

async fn answers(repo: &Path) -> BTreeMap<String, String> {
    let router = gitterrace_web::app_router(repo).expect("router");
    oracle::ready(&router).await;
    let mut out = BTreeMap::new();
    for t in targets() {
        for q in ["period=all", "period=1y", "period=all&bots=1"] {
            let key = format!("{t}?{q}");
            let (status, body) = page(&router, &key).await;
            out.insert(key, format!("{status}\n{}", oracle::normalize(&body)));
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_are_the_same_built_fresh_or_by_appends() {
    let tmp = tempfile::tempdir().expect("tmp");
    let r = RenameChain::build(tmp.path());
    let cache = r.repo.join(".git/gitterrace");
    let fresh = {
        let repo = gitterrace_git::Repo::discover(&r.repo).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        index.update(&repo, "HEAD", &|_| {}).expect("build");
        answers(&r.repo).await
    };
    std::fs::remove_dir_all(&cache).expect("drop the cache");
    {
        let repo = gitterrace_git::Repo::discover(&r.repo).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let ids = git(
            &r.repo,
            &["rev-list", "--first-parent", "--reverse", "HEAD"],
        );
        for id in ids.lines().step_by(4) {
            index.update(&repo, id, &|_| {}).expect("append");
        }
        index.update(&repo, "HEAD", &|_| {}).expect("append");
        let segs = std::fs::read_dir(&cache)
            .expect("cache")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("seg-"))
            .count();
        assert!(segs > 1, "built by appends, {segs} segments");
    }
    let appended = answers(&r.repo).await;
    assert_eq!(fresh.len(), appended.len());
    let bad: Vec<&String> = fresh
        .iter()
        .filter(|(k, v)| appended.get(*k) != Some(v))
        .map(|(k, _)| k)
        .collect();
    assert!(bad.is_empty(), "differ: {bad:?}");
}
