//! Every page marks the top tab of the section it belongs to: file views under Code, commit
//! views under Commits, refs under Branches & Tags, and only the insights pages under Insights.
//! A page built through a shared helper inherits that helper's tab unless it names its own.

use gitterrace_testkit::{Fixture, oracle, page};

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
