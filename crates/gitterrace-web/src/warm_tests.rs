//! The warm-up registry covers every insights page, and what it builds is what the first
//! visitor after ready reads.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use gitterrace_config::Feature;

use crate::warm::WARMERS;
use crate::wiring_tests::{app, repo};

/// Gated routes no warmer serves, each with why it may start cold.
const COLD_OK: &[(&str, &str)] = &[
    (
        "/api/v1/blame",
        "the API blames through its own handle, one file a client names",
    ),
    (
        "/api/v1/contributors",
        "the API reads the page's shared tables and builds nothing of its own",
    ),
    ("/api/v1/insights", "an index of names, no history read"),
    (
        "/api/v1/insights/{name}",
        "a client asks for one insight with its own filter, which no warm-up can guess",
    ),
];

/// Guard class: a page gated on the index that the warm-up never builds, whether a new
/// insights route or a warmer dropped from the registry. Every route of the insights and
/// blame features, and every page the export seeds, is a warmer's or says why it starts cold.
#[test]
fn every_gated_page_has_a_warmer() {
    let warmed: Vec<&str> = WARMERS
        .iter()
        .flat_map(|w| w.routes.iter().copied())
        .collect();
    let gated = crate::features::FEATURE_ROUTES
        .iter()
        .filter(|(f, _)| matches!(f, Feature::Insights | Feature::Blame))
        .flat_map(|(_, routes)| routes.iter().copied())
        .chain(crate::WARM_PAGES);
    let mut bare = Vec::new();
    for route in gated {
        if !warmed.contains(&route) && !COLD_OK.iter().any(|(r, _)| *r == route) {
            bare.push(route);
        }
    }
    assert!(
        bare.is_empty(),
        "no warmer and no reason to start cold: {bare:?}"
    );
    for (route, _) in COLD_OK {
        assert!(
            !warmed.contains(route),
            "{route} is warmed: drop it from COLD_OK"
        );
    }
}

/// Guard class: any warm-up that runs after ready, or warms a page other than the one a
/// visitor gets. Once the refresher reports ready, the first request of every switched-on
/// warm page is a memo hit: no render, and for languages no chain walk.
#[tokio::test]
async fn the_first_page_after_ready_is_served_from_the_warm_up() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = app(&repo(dir.path(), "r"), gitterrace_config::Config::default());
    tokio::spawn(crate::app::keep_index_fresh_supervised(Arc::clone(&a)));
    gitterrace_testkit::wait::until("ready", || async { a.ready() }).await;
    let router = crate::routes::router(Arc::clone(&a));
    let (renders, walks) = (
        a.renders.load(Ordering::Relaxed),
        a.chain_walks.load(Ordering::Relaxed),
    );
    assert!(renders > 0, "the warm-up rendered nothing before ready");
    for page in crate::features::warm(&a.cfg(), Some(&a.name)) {
        let (status, body) = gitterrace_testkit::get(&router, page).await;
        assert_eq!(status, 200, "{page}: {body}");
        assert_eq!(
            a.renders.load(Ordering::Relaxed),
            renders,
            "{page} rendered cold"
        );
    }
    assert_eq!(
        a.chain_walks.load(Ordering::Relaxed),
        walks,
        "a chain walk after ready"
    );
}
