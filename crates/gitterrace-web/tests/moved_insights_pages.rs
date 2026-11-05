//! Commit types and code frequency became sections of Activity, patterns a section of Hotspots.
//! No function is lost: every old URL answers with a permanent redirect to its section with the
//! query kept, every section and chart of the old pages renders on its new page, and Compare
//! stays reachable from the navigation.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, base, oracle, page};

/// Old path, new path and section id, as the server must answer them.
const MOVED: [(&str, &str, &str); 3] = [
    ("/stats/types", "/stats", "types"),
    ("/stats/frequency", "/stats", "frequency"),
    ("/stats/patterns", "/stats/hotspots", "patterns"),
];

/// Every section id and heading of the old pages, on the page that holds it now. Code age and
/// release cadence are named on their own: they cover all history and were easy to drop.
const SECTIONS: [(&str, &str, &str); 14] = [
    ("/stats", "types", "Commit types"),
    ("/stats", "types", "Types"),
    ("/stats", "types", "Type by scope"),
    ("/stats", "types", "Directories touched"),
    ("/stats", "types", "Files touched most"),
    ("/stats", "commits", "Commits"),
    ("/stats", "frequency", "Code frequency"),
    ("/stats", "frequency", "Additions and deletions per"),
    ("/stats", "frequency", "Growing or reworking?"),
    ("/stats/hotspots", "patterns", "Patterns"),
    ("/stats/hotspots", "coupled", "Coupled files"),
    ("/stats/hotspots", "code-age", "Code age"),
    ("/stats/hotspots", "sizes", "Commit sizes"),
    ("/stats/hotspots", "releases", "Releases"),
];

fn fixture(dir: &std::path::Path) -> Fixture {
    let mut f = Fixture::init(dir)
        .write("src/a.rs", "fn main() {}\n")
        .write("src/b.rs", "fn b() {}\n")
        .commit("feat(core): first");
    for n in 0..4 {
        f = f
            .write("src/a.rs", &format!("fn main() {{ {n} }}\n"))
            .write("src/b.rs", &format!("fn b() {{ {n} }}\n"))
            .commit("fix(core): together");
    }
    f.git(&["tag", "v1"]);
    f.write("docs/x.md", "# x\n").commit("docs: add")
}

/// The `location` header of `uri` and its status.
async fn location(router: &axum::Router, uri: &str) -> (u16, String) {
    use tower::ServiceExt;
    let req = axum::http::Request::get(uri)
        .header("host", "localhost")
        .body(axum::body::Body::empty())
        .expect("request");
    let res = router.clone().oneshot(req).await.expect("answer");
    let loc = res
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    (res.status().as_u16(), loc)
}

#[tokio::test]
async fn every_old_url_redirects_with_its_query() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = fixture(dir.path());
    let router = gitterrace_web::app_router(f.path()).expect("router");
    // The pages live under the repository's own prefix, and so do their new homes.
    let root = base(&router).await;
    for (from, to, anchor) in MOVED {
        for q in [
            "",
            "period=1y",
            "period=3m&type=fix&author=a%40b.c&bots=1&page=1",
        ] {
            let uri = if q.is_empty() {
                format!("{root}{from}")
            } else {
                format!("{root}{from}?{q}")
            };
            let (status, loc) = location(&router, &uri).await;
            assert_eq!(status, 308, "{uri}");
            let want = if q.is_empty() {
                format!("{root}{to}#{anchor}")
            } else {
                format!("{root}{to}?{q}#{anchor}")
            };
            assert_eq!(loc, want, "{uri}");
        }
    }
}

#[tokio::test]
async fn every_old_section_renders_on_its_new_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = fixture(dir.path());
    let router = gitterrace_web::app_router(f.path()).expect("router");
    oracle::ready(&router).await;
    let mut fails = Vec::new();
    for (p, id, heading) in SECTIONS {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        if !body.contains(&format!("id=\"{id}\"")) {
            fails.push(format!("{p}: no section #{id}"));
        }
        if !body.contains(&format!("<h2>{heading}")) && !body.contains(&format!(">{heading}</h2>"))
        {
            fails.push(format!("{p}: no heading {heading}"));
        }
    }
    // The charts themselves, not only their headings.
    let (_, hot) = page(&router, "/stats/hotspots").await;
    let age = hot.split("id=\"code-age\"").nth(1).unwrap_or_default();
    if !age
        .split("id=\"sizes\"")
        .next()
        .unwrap_or_default()
        .contains("<svg")
    {
        fails.push("code age has no chart".to_owned());
    }
    let rel = hot.split("id=\"releases\"").nth(1).unwrap_or_default();
    if !rel.contains("v1") || !rel.contains("since the last") {
        fails.push("release cadence lost its tag or its cadence line".to_owned());
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[tokio::test]
async fn five_insights_tabs_and_compare_sits_under_commits() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = fixture(dir.path());
    let router = gitterrace_web::app_router(f.path()).expect("router");
    oracle::ready(&router).await;
    let root = base(&router).await;
    let (_, body) = page(&router, "/stats").await;
    let tabs = body
        .split("<nav class=\"tabs\">")
        .nth(1)
        .and_then(|t| t.split("</nav>").next())
        .expect("tab row");
    for gone in ["Commit types", "Code frequency", "Patterns", "More"] {
        assert!(!tabs.contains(gone), "{gone} is still a tab");
    }
    for kept in [
        "Activity",
        "Contributors",
        "Hotspots",
        "Languages",
        "Ownership",
    ] {
        assert!(tabs.contains(&format!(">{kept}</a>")), "{kept} tab missing");
    }
    // Compare sits under Commits: no insights tab, a link on the log, Commits marked there.
    assert!(
        !tabs.contains("Compare"),
        "Compare is still an insights tab"
    );
    let (_, log) = page(&router, "/log/HEAD").await;
    assert!(
        log.contains(&format!("href=\"{root}/compare?")),
        "the log does not link to Compare"
    );
    let (_, cmp) = page(&router, "/compare").await;
    let nav = cmp
        .split("<nav class=\"tabnav\">")
        .nth(1)
        .unwrap_or_default();
    let on = nav.split("class=\"on\"").nth(1).unwrap_or_default();
    assert!(on.starts_with(" aria-current=\"page\">Commits<"), "{nav}");
}
