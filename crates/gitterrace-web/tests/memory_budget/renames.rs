//! Idle memory on a history of many renamed paths.

use std::io::Write;

use gitterrace_testkit::synthetic;

use crate::{BUDGET_MB, MB, Server, apply_and_assert_allocator, serve_if_child};

/// Idle after hotspots, ownership and stats on a history of 200k paths that are 20k files
/// renamed nine times stays inside the budget: section 7 (about 0.9 MB here) is read through
/// the block cache, never held whole.
#[test]
fn idle_stays_in_budget_with_many_renames() {
    if serve_if_child() {
        return;
    }
    apply_and_assert_allocator();
    if cfg!(not(any(target_os = "macos", target_os = "linux"))) {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    synthetic::build_renames(dir.path(), 20_000, 9);
    let s = Server::start(dir.path(), "renames::idle_stays_in_budget_with_many_renames");
    let base = s.base();
    s.wait_ready(&base);
    s.wait_released(&base, 0);
    let before = s.releases(&base);
    for p in ["/stats/hotspots", "/stats/ownership", "/stats"] {
        assert_eq!(s.get(&format!("{base}{p}?period=all")).0, 200, "{p}");
    }
    s.wait_released(&base, before);
    let idle = s.dirty();
    writeln!(std::io::stderr(), "many renames: idle {} MB", idle / MB).expect("stderr");
    assert!(idle <= BUDGET_MB * MB, "idle {} MB, budget {BUDGET_MB} MB", idle / MB);
}
