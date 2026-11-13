//! Scripts behind a gesture are not on the page: `app.js` adds them on first use, and the
//! server renders the triggers they bind so the bar does not move when they arrive.

use gitterrace_config::Config;
use gitterrace_testkit::{Fixture, page as get};

use crate::wiring_tests::router;

/// Every `/static/` script and stylesheet a page links.
fn assets(body: &str) -> Vec<&str> {
    ["src=\"/static/", "href=\"/static/"]
        .iter()
        .flat_map(|m| body.split(m).skip(1))
        // The name without its `?v=`, so a lazy asset matches whatever build linked it.
        .filter_map(|s| s.split(['"', '?']).next())
        .filter(|s| {
            std::path::Path::new(s)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("js") || e.eq_ignore_ascii_case("css"))
        })
        .collect()
}

#[tokio::test]
async fn code_and_commit_pages_link_no_script_or_style_that_loads_on_first_use() {
    let dir = tempfile::tempdir().expect("tmp");
    let fx = Fixture::init(&dir.path().join("r"))
        .write("src/a.rs", "fn a() {}\n")
        .commit("feat: a");
    let sha = fx.git(&["rev-parse", "HEAD"]);
    let app = router(fx.path(), Config::default());
    let commit = format!("/commit/{}", sha.trim());
    for path in [
        "/blob/HEAD/src/a.rs",
        commit.as_str(),
        "/log/HEAD",
        "/search?q=a",
    ] {
        let (status, body) = get(&app, path).await;
        assert_eq!(status, 200, "{path}");
        let linked = assets(&body);
        for lazy in crate::LAZY_ASSETS {
            let name = lazy.trim_start_matches("/static/");
            assert!(!linked.contains(&name), "{path} links {name}: {linked:?}");
        }
        for page_only in ["settings.css", "insights.css", "graph.css"] {
            assert!(!linked.contains(&page_only), "{path} links {page_only}");
        }
        assert!(
            body.contains("id=\"actpage\""),
            "{path}: the page trigger is rendered"
        );
    }
    let (_, body) = get(&app, &commit).await;
    assert!(
        body.contains("id=\"actcommit\""),
        "the commit trigger is rendered"
    );
}

#[tokio::test]
async fn every_asset_loaded_on_first_use_is_served() {
    let dir = tempfile::tempdir().expect("tmp");
    let fx = Fixture::init(&dir.path().join("r"))
        .write("a", "a\n")
        .commit("feat: a");
    let app = router(fx.path(), Config::default());
    for lazy in crate::LAZY_ASSETS {
        assert_eq!(get(&app, lazy).await.0, 200, "{lazy}");
    }
}
