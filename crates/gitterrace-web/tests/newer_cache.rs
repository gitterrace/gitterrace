//! A cache written by a newer build is served around, never read, never overwritten: every page
//! and API path answers below 500, the insights pages say a newer build wrote the cache, and the
//! cache folder is byte for byte what it was.
#![expect(clippy::expect_used, reason = "test setup")]

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use gitterrace_testkit::{Fixture, base, get};

fn folder(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "lock")
        .map(|n| {
            let b = std::fs::read(dir.join(&n)).expect("read");
            (n, b)
        })
        .collect()
}

fn links(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for attr in ["href=\"", "src=\""] {
        for (i, _) in body.match_indices(attr) {
            let rest = &body[i + attr.len()..];
            let v = &rest[..rest.find('"').unwrap_or(0)];
            if v.starts_with('/') && !v.starts_with("//") {
                out.push(v.replace("&amp;", "&"));
            }
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_cache_serves_every_page_without_a_500_and_stays_untouched() {
    let tmp = tempfile::tempdir().expect("tmp");
    let repo = Fixture::standard(&tmp.path().join("repo")).path().to_owned();
    let cache = repo.join(".git/gitterrace");
    {
        let r = gitterrace_git::Repo::discover(&repo).expect("repo");
        let index = gitterrace_index::Index::open(&r).expect("index");
        index.update(&r, "HEAD", &|_| {}).expect("build");
    }
    let newer = gitterrace_index::FORMAT_VERSION.parse::<u64>().expect("numeric") + 1;
    std::fs::write(cache.join("VERSION"), format!("v{newer}\n")).expect("w");
    std::fs::write(cache.join("future.gts"), b"a newer build's file").expect("w");
    let before = folder(&cache);

    let router = gitterrace_web::app_router(&repo).expect("router");
    let prefix = base(&router).await;
    // Give the indexer loop time to try (and refuse) the cache before the sweep.
    gitterrace_testkit::wait::pause(std::time::Duration::from_millis(500)).await;

    let mut todo: Vec<String> = gitterrace_web::WARM_PAGES
        .iter()
        .map(|p| format!("{prefix}{p}"))
        .chain(gitterrace_api::ENDPOINTS.iter().map(|e| {
            format!(
                "{prefix}/api/v1{}?path=src/a.rs",
                e.path.replace("{id}", "HEAD").replace("{name}", "summary")
            )
        }))
        .collect();
    let mut seen = HashSet::new();
    let mut errors = Vec::new();
    while let Some(u) = todo.pop() {
        if !seen.insert(u.clone()) || seen.len() > 300 {
            continue;
        }
        let (s, body) = get(&router, &u).await;
        if s >= 500 {
            errors.push(format!("{u} -> {s}"));
        }
        if s == 200 {
            todo.extend(
                links(&body)
                    .into_iter()
                    .map(|l| l.split('#').next().unwrap_or_default().to_owned()),
            );
        }
    }
    assert!(seen.len() > 20, "the sweep reached only {} paths", seen.len());
    assert!(errors.is_empty(), "5xx over a newer cache:\n{}", errors.join("\n"));

    for p in gitterrace_web::WARM_PAGES.iter().filter(|p| p.starts_with("/stats")) {
        let (s, body) = get(&router, &format!("{prefix}{p}")).await;
        assert_eq!(s, 200, "{p}");
        assert!(
            body.contains("written by a newer gitterrace"),
            "{p} does not say the cache is newer"
        );
    }
    assert!(folder(&cache) == before, "the newer cache was changed");
}
