//! Every page a fixture repository links to passes the static screen reader checks.
//!
//! The checks are `gitterrace_testkit::a11y`. The crawl starts at the overview and follows
//! every link inside the repository, so a new page is checked the day it is linked. Seeds add
//! what no link reaches: a search with hits, a compare and the working tree.

use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use gitterrace_testkit::{Fixture, base, get};

fn links(html: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in html.match_indices("href=\"") {
        let rest = &html[i + 6..];
        let v = &rest[..rest.find('"').unwrap_or(0)];
        let v = v
            .split('#')
            .next()
            .unwrap_or_default()
            .replace("&amp;", "&");
        // Raw files and assets are no pages; the API answers JSON.
        let skip = ["/raw/", "/static/", "/api/", "/_"]
            .iter()
            .any(|s| v.contains(s));
        if v.starts_with(prefix) && !skip {
            out.push(v);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn every_linked_page_reads_well() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::standard(&dir.path().join("repo"))
        .write(
            "docs/page.md",
            "---\ntitle: Front\n---\n# Page\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        )
        .commit("docs: a page with front matter and a table")
        .tag("v1", "Release one")
        .branch("side")
        .write("src/a.rs", "fn main() {\n    side();\n}\n")
        .commit("feat: on side")
        .checkout("main")
        .write("src/b.rs", "fn b() {}\n")
        .commit("feat(core): third")
        .write("src/a.rs", "fn main() {}\n// dirty\n");
    let f = f.stash("wip");
    let f = f.write("README.md", "# Hi\n\nchanged\n");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    let at = base(&router).await;
    for _ in 0..200 {
        let (_, body) = get(&router, &format!("{at}/stats")).await;
        if !body.contains("data-progress") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut todo: Vec<String> = [
        "/",
        "/search?q=main",
        "/search?q=first&kind=commits",
        "/compare?base=v1&head=main",
        "/worktree",
    ]
    .iter()
    .map(|p| format!("{at}{p}"))
    .collect();
    let mut seen = HashSet::new();
    let mut bad = BTreeSet::new();
    while let Some(u) = todo.pop() {
        if !seen.insert(u.clone()) || seen.len() > 400 {
            continue;
        }
        let (status, body) = get(&router, &u).await;
        if status != 200 || !body.contains("<html") {
            continue;
        }
        for p in gitterrace_testkit::a11y::problems(&body) {
            bad.insert(format!("{u}: {p}"));
        }
        todo.extend(links(&body, &at));
    }
    assert!(seen.len() > 30, "crawl too shallow: {}", seen.len());
    assert!(
        bad.is_empty(),
        "pages a screen reader cannot read:\n{}",
        bad.into_iter().collect::<Vec<_>>().join("\n")
    );
}
