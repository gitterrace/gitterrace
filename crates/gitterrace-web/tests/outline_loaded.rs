//! Every page with an "On this page" outline gets the script that wires its filter, keys and
//! highlight. app.js loads it on demand for any `[data-outline]`, so a page needs app.js and
//! the loader must still name that selector. Without both, the filter never appears.

use gitterrace_testkit::matrix::Mailmap;
use gitterrace_testkit::{oracle, page};

/// The script's URL up to its build version, which moves with any asset edit.
const SCRIPT: &str = "src=\"/static/app.js?v=";

/// Pages whose HTML has an outline but not the script.
fn missing(pages: &[(String, String)]) -> Vec<&str> {
    pages
        .iter()
        .filter(|(_, body)| body.contains("data-outline") && !body.contains(SCRIPT))
        .map(|(p, _)| p.as_str())
        .collect()
}

#[test]
fn the_check_sees_an_outline_without_its_script() {
    let bad = vec![("/x".to_owned(), "<nav data-outline></nav>".to_owned())];
    assert_eq!(missing(&bad), ["/x"]);
    let good = vec![(
        "/x".to_owned(),
        format!("<nav data-outline></nav><script {SCRIPT}1\">"),
    )];
    assert!(missing(&good).is_empty());
}

/// The loader in app.js fetches outline.js for every outline on the page.
#[test]
fn app_js_loads_the_outline_script_for_an_outline() {
    let app = include_str!("../static/app.js");
    assert!(
        app.contains("[data-outline]") && app.contains("gtLazy(\"outline\")"),
        "app.js no longer loads outline.js for [data-outline]"
    );
}

#[test]
fn every_page_with_an_outline_loads_the_outline_script() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let dir = tempfile::tempdir().expect("tmp");
    let (repo, _) = oracle::matrix_repo(dir.path(), Mailmap::AsIs);
    let pages = rt.block_on(async {
        let router = gitterrace_web::app_router(&repo).expect("router");
        oracle::ready(&router).await;
        let mut paths: Vec<String> = gitterrace_web::WARM_PAGES
            .iter()
            .map(|p| (*p).to_owned())
            .collect();
        paths.extend(["/".to_owned(), "/blob/HEAD/docs/x.md".to_owned()]);
        let mut out = Vec::new();
        for p in paths {
            let (_, body) = page(&router, &p).await;
            out.push((p, body));
        }
        out
    });
    assert!(
        pages.iter().any(|(_, b)| b.contains("data-outline")),
        "no page has an outline: the check proves nothing"
    );
    assert!(
        missing(&pages).is_empty(),
        "outline without {SCRIPT}: {:?}",
        missing(&pages)
    );
}
