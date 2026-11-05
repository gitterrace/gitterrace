//! The incident behind this test: a server already RUNNING while another build moves
//! the cache to a newer format and a segment goes bad underneath it. No page, API path or
//! repeat visit answers 5xx: the index says why or asks for an index instead.

#![expect(clippy::expect_used, reason = "test setup")]

use gitterrace_testkit::{Fixture, base, get};

/// Asks only `/api/v1/repo` until it answers 200 again, at most `limit`: no page is asked
/// meanwhile, so a page's own rebuild cannot stand in for the API's. Every answer before is a
/// 503 with `Retry-After`. Returns how long it took and anything else seen.
async fn until_api_answers(
    router: &axum::Router,
    prefix: &str,
    limit: std::time::Duration,
) -> (Option<std::time::Duration>, Vec<String>) {
    let t0 = std::time::Instant::now();
    let mut errors = Vec::new();
    let mut round = 0;
    while t0.elapsed() < limit {
        let u = format!("{prefix}/api/v1/repo?author=probe{round}");
        let (status, retry) = api(router, &u).await;
        match status {
            200 => return (Some(t0.elapsed()), errors),
            503 if retry.is_some() => {}
            s => errors.push(format!("{u} -> {s}, Retry-After {retry:?}")),
        }
        round += 1;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    (None, errors)
}

/// Status and `Retry-After` of one API answer.
async fn api(router: &axum::Router, uri: &str) -> (u16, Option<String>) {
    use tower::ServiceExt as _;
    let req = axum::http::Request::get(uri)
        .header("host", "localhost")
        .body(axum::body::Body::empty())
        .expect("request");
    let res = router.clone().oneshot(req).await.expect("response");
    let retry = res
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    (res.status().as_u16(), retry)
}

/// Flips one byte in the middle of every segment.
fn corrupt(cache: &std::path::Path) {
    let segs = segments(cache);
    assert!(!segs.is_empty(), "no segment to corrupt");
    for p in segs {
        let mut b = std::fs::read(&p).expect("read");
        let mid = b.len() / 2;
        b[mid] ^= 0xff;
        std::fs::write(&p, b).expect("corrupt");
    }
}

/// How long the API may answer 503 after its cache went bad. The fixture answers again after
/// 80 to 210 ms on a machine at load 130 to 280, so this is fifty times the slowest seen.
const REBUILT_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

/// Segments corrupted under a running server: never a 500, a 503 with `Retry-After` while the
/// cache is rebuilt, then 200 again, asked through the API alone. The API hands the failure to
/// the server's one rebuild; that it starts a rebuild with nothing else asking is pinned in
/// the API crate (`an_unreadable_cache_is_rebuilt_by_the_api_alone`), red without its trigger.
#[test]
fn a_running_server_rebuilds_a_cache_corrupted_underneath_for_the_api_too() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let tmp = tempfile::tempdir().expect("tmp");
        let (repo, router, prefix) = built(tmp.path()).await;
        let cache = repo.join(".git/gitterrace");
        let (s, _) = api(&router, &format!("{prefix}/api/v1/repo")).await;
        assert_eq!(s, 200, "the API answers before the corruption");
        corrupt(&cache);
        let (took, errors) = until_api_answers(&router, &prefix, REBUILT_WITHIN).await;
        assert!(
            errors.is_empty(),
            "while rebuilding:\n{}",
            errors.join("\n")
        );
        assert!(
            took.is_some(),
            "the API answers 200 again after the rebuild"
        );
        // The pages answer from the rebuilt cache too, and never with a 5xx.
        for p in ["/", "/stats", "/log/HEAD"] {
            let (s, _) = get(&router, &format!("{prefix}{p}?author=after")).await;
            assert!(s < 500, "{p} -> {s} after the rebuild");
        }
    });
}

/// The incident behind this test: a server already RUNNING while another build moves the
/// cache to a newer format and a segment goes bad underneath it. A newer build's cache is
/// never written, so it is not rebuilt: no page or API answer is a 500, every API answer is a
/// 503 `index_newer` with `Retry-After`, and the cache stays byte for byte as it was.
#[test]
fn a_running_server_survives_its_cache_being_rewritten_underneath() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let tmp = tempfile::tempdir().expect("tmp");
        let (repo, router, prefix) = built(tmp.path()).await;
        let cache = repo.join(".git/gitterrace");
        for _ in 0..40 {
            let (s, _) = get(&router, &format!("{prefix}/stats")).await;
            assert!(s < 500, "/stats -> {s} before the rewrite");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        // Another build rewrites the cache while the server runs.
        let newer = gitterrace_index::FORMAT_VERSION
            .parse::<u64>()
            .expect("numeric")
            + 1;
        std::fs::write(cache.join("VERSION"), format!("{newer}\n")).expect("w");
        corrupt(&cache);
        let before: Vec<_> = segments(&cache)
            .iter()
            .map(|p| std::fs::read(p).expect("read"))
            .collect();
        let mut errors = Vec::new();
        for round in 0..20 {
            for p in ["/", "/stats", "/log/HEAD"] {
                let u = format!("{prefix}{p}?author=probe{round}");
                let (s, _) = get(&router, &u).await;
                if s >= 500 {
                    errors.push(format!("round {round}: {u} -> {s}"));
                }
            }
            let u = format!("{prefix}/api/v1/repo?author=probe{round}");
            let (status, retry) = api(&router, &u).await;
            let (_, body) = get(&router, &u).await;
            if status != 503 || retry.is_none() || !body.contains("index_newer") {
                errors.push(format!(
                    "round {round}: {u} -> {status}, Retry-After {retry:?}"
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(
            errors.is_empty(),
            "after the rewrite:\n{}",
            errors.join("\n")
        );
        let after: Vec<_> = segments(&cache)
            .iter()
            .map(|p| std::fs::read(p).expect("read"))
            .collect();
        assert!(before == after, "a newer build's cache was written");
    });
}

async fn built(tmp: &std::path::Path) -> (std::path::PathBuf, axum::Router, String) {
    let repo = Fixture::standard(&tmp.join("repo")).path().to_owned();
    {
        let r = gitterrace_git::Repo::discover(&repo).expect("repo");
        let index = gitterrace_index::Index::open(&r).expect("index");
        index.update(&r, "HEAD", &|_| {}).expect("build");
    }
    let router = gitterrace_web::app_router(&repo).expect("router");
    let prefix = base(&router).await;
    (repo, router, prefix)
}

fn segments(cache: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(cache)
        .expect("dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "gts")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("seg-"))
        })
        .collect()
}

/// A segment truncated or filled with garbage under a running server is detected while
/// serving and rebuilt: no 5xx, and the page ends up answering from a sound cache again.
/// Red without `check_unreadable`.
#[test]
fn a_truncated_or_garbage_segment_under_a_running_server_is_rebuilt() {
    for garbage in [false, true] {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            let tmp = tempfile::tempdir().expect("tmp");
            let (repo, router, prefix) = built(tmp.path()).await;
            let cache = repo.join(".git/gitterrace");
            // Warm: the stats page reads the segments, so the server holds them.
            for i in 0..5 {
                let (s, _) = get(&router, &format!("{prefix}/stats?author=warm{i}")).await;
                assert!(s < 500, "/stats -> {s} while warming");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            let segs = segments(&cache);
            assert!(!segs.is_empty(), "no segment to damage");
            let before = segs.len();
            for p in &segs {
                let b = std::fs::read(p).expect("read");
                let damaged = if garbage {
                    vec![0xa5; b.len()]
                } else {
                    b[..20.min(b.len())].to_vec()
                };
                std::fs::write(p, damaged).expect("damage");
            }
            let damaged: Vec<Vec<u8>> = segs
                .iter()
                .map(|p| std::fs::read(p).expect("read"))
                .collect();
            let mut errors = Vec::new();
            let mut healed = false;
            for round in 0..60 {
                for p in ["/stats", "/", "/log/HEAD"] {
                    let u = format!("{prefix}{p}?author=probe{round}");
                    let (s, _) = get(&router, &u).await;
                    if s >= 500 {
                        errors.push(format!("round {round}: {u} -> {s}"));
                    }
                }
                let now: Vec<Option<Vec<u8>>> = segments(&cache)
                    .iter()
                    .map(|p| std::fs::read(p).ok())
                    .collect();
                if !now.is_empty()
                    && now
                        .iter()
                        .all(|n| n.as_ref().is_some_and(|n| !damaged.contains(n)))
                {
                    healed = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            assert!(errors.is_empty(), "5xx:\n{}", errors.join("\n"));
            assert!(
                healed,
                "the damaged cache (garbage: {garbage}) was never rebuilt; had {} segments; now {:?}",
                before,
                std::fs::read_dir(&cache).unwrap().flatten().map(|e| (e.file_name(), e.metadata().unwrap().len())).collect::<Vec<_>>()
            );
        });
    }
}

/// A cache dir a newer build owns (the server must treat it as read-only, even though the
/// directory itself is writable) never turns into a 5xx, and a damaged
/// cache that cannot be rebuilt is not replaced by garbage. Red without `check_writable`.
#[test]
fn a_cache_a_newer_build_owns_is_never_written_by_a_running_server() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let tmp = tempfile::tempdir().expect("tmp");
        let (repo, router, prefix) = built(tmp.path()).await;
        let cache = repo.join(".git/gitterrace");
        for i in 0..3 {
            let (s, _) = get(&router, &format!("{prefix}/stats?author=warm{i}")).await;
            assert!(s < 500, "/stats -> {s} while warming");
        }
        let snapshot = |dir: &std::path::Path| {
            let mut v: Vec<_> = std::fs::read_dir(dir)
                .expect("dir")
                .flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    !n.starts_with('.') && n != "lines.json"
                })
                .map(|e| (e.file_name(), std::fs::read(e.path()).unwrap_or_default()))
                .collect();
            v.sort();
            v
        };
        // The server's own startup update must be done (it removes and rewrites `tip`),
        // or its late write would look like a write after the newer build took over.
        for _ in 0..100 {
            if cache.join("tip").exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // A newer build owns the cache and made it read-only.
        let newer = gitterrace_index::FORMAT_VERSION
            .parse::<u64>()
            .expect("numeric")
            + 1;
        std::fs::write(cache.join("VERSION"), format!("{newer}\n")).expect("w");
        let before = snapshot(&cache);
        // HEAD moves, so the running server wants to index: it must not write.
        let fx = Fixture::init(&repo);
        let _fx = fx.write("late.txt", "late\n").commit("feat: late");
        let mut errors = Vec::new();
        for round in 0..30 {
            for p in ["/", "/stats", "/log/HEAD", "/api/v1/repo"] {
                let u = format!("{prefix}{p}?author=ro{round}");
                let (s, _) = get(&router, &u).await;
                if s >= 500 {
                    errors.push(format!("round {round}: {u} -> {s}"));
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let after = snapshot(&cache);
        assert!(errors.is_empty(), "5xx:\n{}", errors.join("\n"));
        let names = |v: &Vec<(std::ffi::OsString, Vec<u8>)>| {
            v.iter()
                .map(|(n, b)| format!("{n:?}:{}", b.len()))
                .collect::<Vec<_>>()
        };
        assert!(
            before == after,
            "the read-only cache was changed:\n{:?}\n{:?}",
            names(&before),
            names(&after)
        );
    });
}

/// A cache written by the format before rename-aware aggregates (checked-in bytes of format
/// 6, segment version 6, never produced by this build) is wiped and rebuilt: every insights page
/// answers 200 or the building page, never a 5xx, and the rebuilt cache is the current format.
/// Red when the previous version is still accepted: the page then reads a cache without lives.
#[test]
fn a_version_6_cache_is_rebuilt_to_7_without_a_500() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let tmp = tempfile::tempdir().expect("tmp");
        let repo = Fixture::standard(&tmp.path().join("repo"))
            .path()
            .to_owned();
        let cache = repo.join(".git/gitterrace");
        let old = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cache-v6");
        std::fs::create_dir_all(&cache).expect("mkdir");
        for e in std::fs::read_dir(&old).expect("fixture").flatten() {
            std::fs::copy(e.path(), cache.join(e.file_name())).expect("copy");
        }
        let seg = std::fs::read(cache.join("seg-1-0.gts")).expect("v6 segment");
        assert_eq!(
            &seg[..8],
            b"GTS1\x06\0\0\0",
            "the fixture holds segment version 6"
        );
        assert_eq!(
            std::fs::read_to_string(cache.join("VERSION")).expect("v"),
            "6\n"
        );

        let router = gitterrace_web::app_router(&repo).expect("router");
        let prefix = base(&router).await;
        gitterrace_testkit::oracle::ready(&router).await;
        let mut errors = Vec::new();
        for p in gitterrace_web::WARM_PAGES
            .iter()
            .chain(&["/", "/api/v1/insights/hotspots"])
        {
            let (s, _) = get(&router, &format!("{prefix}{p}")).await;
            if s != 200 {
                errors.push(format!("{p} -> {s}"));
            }
        }
        assert!(errors.is_empty(), "{}", errors.join("\n"));
        let version = std::fs::read_to_string(cache.join("VERSION")).expect("version");
        assert_eq!(version.trim(), gitterrace_index::FORMAT_VERSION);
        let rebuilt = segments(&cache);
        assert!(!rebuilt.is_empty(), "no segment after the rebuild");
        for p in rebuilt {
            let b = std::fs::read(&p).expect("read");
            assert_eq!(
                &b[..8],
                b"GTS1\x07\0\0\0",
                "{} is segment version 7",
                p.display()
            );
        }
    });
}
