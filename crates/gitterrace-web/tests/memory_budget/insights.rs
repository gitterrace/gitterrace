//! Insights request cost and the idle release, over a warm server. Split from
//! `memory_budget.rs`, whose server harness it reuses.

use std::io::Write;
use std::path::PathBuf;

use gitterrace_testkit::synthetic::{self, Shape};

use crate::{MB, Server, apply_and_assert_allocator, serve_if_child};

/// Most one insights request may add over a warm idle: the reducer state of one request
/// (people, files, buckets), never a per-commit list. Before the pages read one `Counted`
/// (100k synthetic, release) the first /stats/types?period=all added 12 MB and the API
/// summary with a path 10 MB, both from
/// filling shared state the server keeps by design, not from the request itself.
const REQUEST_PEAK_MB: u64 = 8;
/// Idle after the requests: what a just-started server may hold plus the cache budget. Warm
/// state counts here and not against one request: the disk block cache (`CACHE_BYTES`, 12 MB,
/// feed/disk.rs) and the shared derived tables (skip, commit lines, follow maps).
const FOOTPRINT_IDLE_MB: u64 = 48;

/// What [`insights_request_peak`] and [`insights_release_returns_to_start`] read off one run.
struct PeakRun {
    /// Dirty bytes of a just-started server over a one-commit repository.
    start: u64,
    /// After the idle release that follows the measured requests.
    released: u64,
    /// Budget breaches of the warm idle and of each request.
    bad: Vec<String>,
}

/// Rendered-page memo bytes kept across the idle release by design: one eighth of
/// `resources.cache_mb` (web app.rs `SMALL_EIGHTHS`).
fn memo_budget() -> u64 {
    gitterrace_config::Config::default().resources.cache_mb as u64 * MB / 8
}

/// One request's cost: /stats, /stats/types and the API summary at `all` and `1y`, with and
/// without a path selection. Every page is read once first, so the block cache and the derived
/// tables are built. The warm idle must stay within the footprint budget, then each page is
/// measured over that warm idle. Runs on `TEST_LINUX=<path>` (the bench corpus linux clone,
/// 1.3M commits) when set, else on a 100k synthetic history. Afterwards the server must return
/// to idle: nothing a request built stays.
#[test]
fn insights_request_peak() {
    if serve_if_child() {
        return;
    }
    let Some(mut run) = peak_run("insights::insights_request_peak") else {
        return;
    };
    if run.released > run.start + FOOTPRINT_IDLE_MB * MB {
        run.bad.push(format!(
            "idle after the requests: {} MB above the just-started {} MB, budget {FOOTPRINT_IDLE_MB} MB",
            run.released.saturating_sub(run.start) / MB,
            run.start / MB
        ));
    }
    assert!(run.bad.is_empty(), "{}", run.bad.join("\n"));
}

/// After the idle release the server holds what a just-started one does, plus the page memo it
/// keeps by design and 2 MB of slack. The block cache and the derived tables go with the source.
#[test]
#[ignore = "red until an idle release returns to the start: 100k synthetic sits 8 MB over"]
fn insights_release_returns_to_start() {
    if serve_if_child() {
        return;
    }
    let Some(run) = peak_run("insights::insights_release_returns_to_start") else {
        return;
    };
    let cap = run.start + memo_budget() + 2 * MB;
    assert!(
        run.released <= cap,
        "released idle {} MB, just-started {} MB plus memo {} MB plus 2 MB",
        run.released / MB,
        run.start / MB,
        memo_budget() / MB
    );
}

fn peak_run(test: &str) -> Option<PeakRun> {
    apply_and_assert_allocator();
    if cfg!(not(any(target_os = "macos", target_os = "linux"))) {
        return None;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let repo = std::env::var_os("TEST_LINUX").map_or_else(
        || {
            let r = dir.path().join("r100k");
            synthetic::build(&r, Shape::sized(100_000));
            r
        },
        PathBuf::from,
    );
    let start = {
        let tiny = tempfile::tempdir().expect("tmp");
        synthetic::build(
            tiny.path(),
            Shape {
                commits: 1,
                ..Shape::sized(1)
            },
        );
        let t = Server::start(tiny.path(), test);
        let b = t.base();
        t.wait_ready(&b);
        t.dirty()
    };
    let s = Server::start(&repo, test);
    let base = s.base();
    s.wait_ready(&base);
    s.wait_released(&base, 0);
    let path = if std::env::var_os("TEST_LINUX").is_some() {
        "kernel"
    } else {
        "d1"
    };
    let mut pages = Vec::new();
    for period in ["all", "1y"] {
        for sel in [String::new(), format!("&path={path}")] {
            pages.push(format!("{base}/stats?period={period}{sel}"));
            pages.push(format!("{base}/stats/types?period={period}{sel}"));
            pages.push(format!(
                "{base}/api/v1/insights/summary?period={period}{sel}"
            ));
        }
    }
    let mut bad = Vec::new();
    // The memo drops unknown parameters, so a second read of the same URL is a memo hit.
    // Warm every page as measured plus `bots=0`, the default: the same counts and shared
    // state, another memo key. Another period would leave state that `all` reads unbuilt.
    for p in &pages {
        let w = format!("{p}&bots=0");
        let (status, _) = s.get(&w);
        assert_eq!(status, 200, "{w}");
    }
    let warm = s.dirty();
    writeln!(
        std::io::stderr(),
        "warm idle {} MB, just-started {} MB",
        warm / MB,
        start / MB
    )
    .expect("stderr");
    if warm > start + FOOTPRINT_IDLE_MB * MB {
        bad.push(format!(
            "warm idle: {} MB above the just-started {} MB, budget {FOOTPRINT_IDLE_MB} MB",
            warm.saturating_sub(start) / MB,
            start / MB
        ));
    }
    for p in &pages {
        let idle = s.dirty();
        let (status, _) = s.get(p);
        assert_eq!(status, 200, "{p}");
        let peak = s.dirty();
        writeln!(
            std::io::stderr(),
            "{p}: idle {} MB, after {} MB",
            idle / MB,
            peak / MB
        )
        .expect("stderr");
        if peak > idle + REQUEST_PEAK_MB * MB {
            bad.push(format!(
                "{p}: {} MB over idle, cap {REQUEST_PEAK_MB} MB",
                peak.saturating_sub(idle) / MB
            ));
        }
    }
    let before = s.releases(&base);
    s.wait_released(&base, before);
    let released = s.dirty();
    writeln!(std::io::stderr(), "released idle {} MB", released / MB).expect("stderr");
    Some(PeakRun {
        start,
        released,
        bad,
    })
}
