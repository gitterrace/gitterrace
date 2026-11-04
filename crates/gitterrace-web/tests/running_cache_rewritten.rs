//! A server already RUNNING while another gitterrace build moves
//! the cache to a newer format and a segment goes bad underneath it. No page, API path or
//! repeat visit answers 5xx: the index says why or asks for an index instead.

use gitterrace_testkit::{Fixture, base, get};

#[test]
fn a_running_server_survives_its_cache_being_rewritten_underneath() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let tmp = tempfile::tempdir().expect("tmp");
        let repo = Fixture::standard(&tmp.path().join("repo"))
            .path()
            .to_owned();
        let cache = repo.join(".git/gitterrace");
        {
            let r = gitterrace_git::Repo::discover(&repo).expect("repo");
            let index = gitterrace_index::Index::open(&r).expect("index");
            index.update(&r, "HEAD", &|_| {}).expect("build");
        }
        let router = gitterrace_web::app_router(&repo).expect("router");
        let prefix = base(&router).await;
        let pages = ["/", "/stats", "/log/HEAD", "/api/v1/repo"];
        // Warm: the server is up and serving the built cache.
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
        let mut hit = 0;
        for e in std::fs::read_dir(&cache).expect("dir").flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("seg-")
                && std::path::Path::new(&name)
                    .extension()
                    .is_some_and(|x| x == "gts")
            {
                let mut b = std::fs::read(e.path()).expect("read");
                let mid = b.len() / 2;
                b[mid] ^= 0xff;
                std::fs::write(e.path(), b).expect("corrupt");
                hit += 1;
            }
        }
        assert!(hit > 0, "no segment to corrupt");

        let mut errors = Vec::new();
        for round in 0..20 {
            for p in pages {
                // A distinct memo-keyed query (author) per round: the page memo cannot
                // answer, so every request must read the rewritten store.
                let u = format!("{prefix}{p}?author=probe{round}");
                let (s, _) = get(&router, &u).await;
                if s >= 500 {
                    errors.push(format!("round {round}: {u} -> {s}"));
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(
            errors.is_empty(),
            "5xx after the rewrite:\n{}",
            errors.join("\n")
        );
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
            let before: Vec<Vec<u8>> = segs
                .iter()
                .map(|p| std::fs::read(p).expect("read"))
                .collect();
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
                before.len(),
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
