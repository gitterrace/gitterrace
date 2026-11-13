//! Every page marks the top tab of the section it belongs to: file views under Code, commit
//! views under Commits, refs under Branches & Tags, and only the insights pages under Insights.
//! A page built through a shared helper inherits that helper's tab unless it names its own.
//! Commit types and code frequency are sections of Activity, patterns a section of Hotspots:
//! each renders there, and the pages they once had answer like any unknown path.

use gitterrace_testkit::{Fixture, base, oracle, page};

/// Pages that are gone: no redirect keeps them, so they must 404 like an unknown path.
const GONE: [&str; 4] = [
    "/stats/types",
    "/stats/frequency",
    "/stats/patterns",
    "/insights",
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

/// History with a tag and files changed together, so every section has data.
fn sections_fixture(dir: &std::path::Path) -> Fixture {
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

/// The label of the one marked entry in the top tab row.
fn marked(body: &str) -> String {
    let nav = body
        .split("<nav class=\"tabnav\">")
        .nth(1)
        .and_then(|n| n.split("</nav>").next())
        .unwrap_or_default();
    nav.split("class=\"on\"")
        .nth(1)
        .and_then(|t| t.split('>').nth(1))
        .and_then(|t| t.split('<').next())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn every_page_marks_its_own_section() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::standard(dir.path());
    let id = f.git(&["rev-parse", "HEAD"]);
    let commit = format!("/commit/{}", id.trim());
    let router = gitterrace_web::app_router(f.path()).expect("router");
    oracle::ready(&router).await;
    let cases = [
        ("/", "Code"),
        ("/tree/HEAD/src", "Code"),
        ("/blob/HEAD/src/a.rs", "Code"),
        ("/blame/HEAD/src/a.rs", "Code"),
        ("/history/src/a.rs", "Code"),
        ("/log/HEAD", "Commits"),
        ("/compare", "Commits"),
        (commit.as_str(), "Commits"),
        ("/refs", "Branches"),
        ("/stats", "Insights"),
        ("/stats/hotspots", "Insights"),
        ("/stats/languages", "Insights"),
    ];
    let mut fails = Vec::new();
    for (uri, want) in cases {
        let (status, body) = page(&router, uri).await;
        let got = marked(&body);
        if status != 200 || got != want {
            fails.push(format!("{uri}: {status}, marks {got:?}, wants {want:?}"));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[tokio::test]
async fn every_old_section_renders_on_its_new_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = sections_fixture(dir.path());
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
    let f = sections_fixture(dir.path());
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
    // Compare sits under Commits: no insights tab and none in the log's toolbar (its one
    // field is the filter), reached from the branch and tag lists, Commits marked there.
    assert!(
        !tabs.contains("Compare"),
        "Compare is still an insights tab"
    );
    let (_, log) = page(&router, "/log/HEAD").await;
    assert!(
        !log.contains(">Compare</a>"),
        "Compare is back in the log toolbar"
    );
    let (_, refs) = page(&router, "/refs").await;
    assert!(
        refs.contains(&format!("href=\"{root}/compare?")),
        "the branch and tag lists do not link to Compare"
    );
    let (_, cmp) = page(&router, "/compare").await;
    let nav = cmp
        .split("<nav class=\"tabnav\">")
        .nth(1)
        .unwrap_or_default();
    let on = nav.split("class=\"on\"").nth(1).unwrap_or_default();
    assert!(on.starts_with(" aria-current=\"page\">Commits<"), "{nav}");
}

/// Class guard: a removed page must not silently resolve, neither as a redirect nor a page.
#[tokio::test]
async fn removed_insights_paths_answer_as_unknown_pages() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = sections_fixture(dir.path());
    let router = gitterrace_web::app_router(f.path()).expect("router");
    oracle::ready(&router).await;
    for p in GONE {
        let (status, body) = page(&router, p).await;
        // The not-found page, never a redirect or the page that took the old one's place. Its
        // next steps follow the path, so the body is checked by what makes it that page.
        assert_eq!(status, 404, "{p}");
        assert!(
            body.contains("Nothing at this address"),
            "{p}: not the not-found page"
        );
        assert!(
            body.contains("class=\"card empty lost\""),
            "{p}: not the not-found page"
        );
    }
}
