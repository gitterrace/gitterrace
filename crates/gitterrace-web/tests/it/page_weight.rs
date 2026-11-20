//! A served page weighs no more than the ratchet registry's budget. The budget only falls, so an
//! inline drawing left unminified, a repeated attribute or a pasted blob fails here.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, oracle};

use crate::common::ready_router;

/// The `web_page_bytes` count of the ratchet registry.
fn budget() -> usize {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reg = std::fs::read_to_string(root.join("schwarzwald/config/ratchet.toml")).expect("reg");
    reg.lines()
        .find_map(|l| l.strip_prefix("web_page_bytes = "))
        .and_then(|v| v.trim().parse().ok())
        .expect("web_page_bytes in [counts]")
}

#[tokio::test]
async fn the_main_pages_stay_within_the_byte_budget() {
    // Relative dates and period windows change the byte count by the day, so the clock is frozen.
    if !oracle::frozen("page_weight::the_main_pages_stay_within_the_byte_budget") {
        return;
    }
    let root = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(&root.path().join("repo"))
        .at("2024-03-01T12:00:00Z")
        .write("README.md", "# Fixture\n\nhello\n")
        .write("a.rs", "fn main() {}\n")
        .commit("feat: first")
        .at("2024-03-02T12:00:00Z")
        .write("a.rs", "fn main() {}\n// two\n")
        .commit("fix: second");
    let router = ready_router(f.path()).await;
    let sha = f.git(&["rev-parse", "HEAD"]);
    let sha = sha.trim();
    let max = budget();
    let mut heavy = Vec::new();
    for uri in ["/", "/blob/HEAD/a.rs", &format!("/commit/{sha}"), "/log/HEAD", "/stats"] {
        // The live version token is a hash whose digit count varies by run: it is blanked.
        let page = oracle::normalize(
            &gitterrace_testkit::wait::page_until(&router, uri, gitterrace_testkit::wait::indexed)
                .await,
        );
        if page.len() > max {
            heavy.push(format!("{uri}: {} bytes", page.len()));
        }
    }
    assert!(heavy.is_empty(), "over the {max} byte budget of web_page_bytes: {heavy:?}");
}
