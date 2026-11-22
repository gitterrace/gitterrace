//! A cache written by a newer build is served around, never read, never overwritten: every page
//! and API path answers below 500, the insights pages say a newer build wrote the cache, and the
//! cache folder is byte for byte what it was.

use gitterrace_testkit::{Fixture, base, get, links};

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_cache_serves_every_page_without_a_500_and_stays_untouched() {
    let tmp = tempfile::tempdir().expect("tmp");
    let repo = Fixture::standard(&tmp.path().join("repo")).path().to_owned();
    let cache = repo.join(".git/gitterrace");
    crate::common::build_index(&repo);
    let newer = gitterrace_index::FORMAT_VERSION.parse::<u64>().expect("numeric") + 1;
    std::fs::write(cache.join("VERSION"), format!("v{newer}\n")).expect("w");
    std::fs::write(cache.join("future.gts"), b"a newer build's file").expect("w");
    let before = crate::common::snapshot(&cache, |n| n == "lock");

    let router = gitterrace_web::app_router(&repo).expect("router");
    let prefix = base(&router).await;
    // Give the indexer loop time to try (and refuse) the cache before the sweep.
    gitterrace_testkit::wait::pause(std::time::Duration::from_millis(500)).await;

    let seeds: Vec<String> = gitterrace_web::WARM_PAGES
        .iter()
        .map(|p| format!("{prefix}{p}"))
        .chain(gitterrace_api::ENDPOINTS.iter().map(|e| {
            format!(
                "{prefix}/api/v1{}?path=src/a.rs",
                e.path.replace("{id}", "HEAD").replace("{name}", "summary")
            )
        }))
        .collect();
    let mut errors = Vec::new();
    let reached = crate::common::crawl(&router, "localhost", seeds, 300, links, |u, s, _| {
        if s >= 500 {
            errors.push(format!("{u} -> {s}"));
        }
    })
    .await;
    assert!(reached > 20, "the sweep reached only {reached} paths");
    assert!(errors.is_empty(), "5xx over a newer cache:\n{}", errors.join("\n"));

    for p in gitterrace_web::WARM_PAGES.iter().filter(|p| p.starts_with("/stats")) {
        let (s, body) = get(&router, &format!("{prefix}{p}")).await;
        assert_eq!(s, 200, "{p}");
        assert!(
            body.contains("written by a newer gitterrace"),
            "{p} does not say the cache is newer"
        );
    }
    assert!(
        crate::common::snapshot(&cache, |n| n == "lock") == before,
        "the newer cache was changed"
    );
}
