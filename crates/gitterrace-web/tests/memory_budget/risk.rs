//! Memory of the knowledge risk reads over synthetic repositories that widen.

use std::io::Write;

use gitterrace_testkit::synthetic::{self, Shape};

use crate::{Server, apply_and_assert_allocator, serve_if_child};

/// Most that a knowledge risk request may add to the live heap at the wide tree over the same
/// request at the narrow one: its heaps hold `limit` rows and its directory table holds
/// people, so only the 2048-file chunk it reads grows. With a 4096-file chunk and the
/// filtered tree collected first, it was 1.4 MB at 50k files on an M1 Max and 3.1 MB on
/// ubuntu CI. Now 0.5 MB on the M1 Max. A map kept per file would add 12 MB or more at
/// 50k files.
const REQUEST_GROWTH_KB: u64 = 1536;

/// The section, the person page and the two API answers, read once each and, for the API ones,
/// measured over a warm idle: the highest live heap they reach above it. The pages carry the
/// Ownership table too, which is not this guard's to bound.
fn peak(files: usize) -> u64 {
    let dir = tempfile::tempdir().expect("tmp");
    let repo = dir.path().join(format!("w{files}"));
    synthetic::build(&repo, Shape::wide(files));
    let s = Server::start(&repo, "risk::knowledge_risk_memory_is_flat_in_tree_size");
    let base = s.base();
    s.wait_ready(&base);
    let who = gitterrace_testkit::git(&repo, &["log", "-1", "--format=%an"]).trim().to_owned();
    let who = who.replace(' ', "%20");
    let shown = [
        format!("{base}/stats/ownership?period=all&drill=d0"),
        format!("{base}/stats/ownership/{who}?period=all"),
    ];
    let pages = [
        format!("{base}/api/v1/insights/knowledge_risk?period=all&limit=500"),
        format!("{base}/api/v1/insights/knowledge_risk?period=all&author={who}"),
    ];
    // The first read builds the shared tables; the measured one is the second, as another key.
    for p in shown.iter().chain(&pages) {
        assert_eq!(s.get(&format!("{p}&bots=0")).0, 200, "{p}");
    }
    let mut top = 0;
    for p in &pages {
        let before = s.heap();
        s.reset_peak();
        assert_eq!(s.get(p).0, 200, "{p}");
        let over = s.heap_peak().saturating_sub(before);
        writeln!(std::io::stderr(), "{files} files: {p}: {} KB", over / 1024).expect("stderr");
        top = top.max(over);
    }
    writeln!(std::io::stderr(), "{files} files: request peak {} KB over idle", top / 1024)
        .expect("stderr");
    top
}

/// The cost of the section does not grow with the tree: 1k against `TEST_MEMORY_WIDE` (default
/// 50k) tracked files.
#[test]
fn knowledge_risk_memory_is_flat_in_tree_size() {
    if serve_if_child() {
        return;
    }
    apply_and_assert_allocator();
    if cfg!(not(any(target_os = "macos", target_os = "linux"))) {
        return;
    }
    let wide: usize =
        std::env::var("TEST_MEMORY_WIDE").ok().and_then(|v| v.parse().ok()).unwrap_or(50_000);
    let (small, large) = (peak(1_000), peak(wide));
    assert!(
        large <= small + REQUEST_GROWTH_KB * 1024,
        "a request peaks {} KB over idle at {wide} files, {} KB at 1000, more than \
         {REQUEST_GROWTH_KB} KB apart",
        large / 1024,
        small / 1024
    );
}
