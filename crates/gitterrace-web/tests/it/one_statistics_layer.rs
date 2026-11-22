//! The web pages and the JSON API answer from one statistics layer. For the same filter and
//! options, every number a page prints must equal the API's: a second implementation on either
//! side drifts, and this test catches it whichever side moves.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, page as get};
use serde_json::Value;

/// Cells of the table rows of `html` holding `marker`, tags stripped, thousands separators gone.
/// A row's header cell (`th`) is its first cell.
fn rows(html: &str, marker: &str) -> Vec<Vec<String>> {
    html.split("<tr")
        .filter(|r| r.contains(marker))
        .map(|r| {
            r.replace("<th", "<td")
                .replace("</th>", "</td>")
                .split("<td")
                .skip(1)
                .map(|c| {
                    let cell = c.split("</td>").next().unwrap_or_default();
                    let cell = cell.split_once('>').map_or("", |(_, rest)| rest);
                    let mut out = String::new();
                    let mut tag = false;
                    for ch in cell.chars() {
                        match ch {
                            '<' => tag = true,
                            '>' if tag => tag = false,
                            _ if !tag => out.push(ch),
                            _ => {}
                        }
                    }
                    out.replace(',', "").trim().to_owned()
                })
                .collect()
        })
        .collect()
}

async fn api(router: &axum::Router, name: &str, query: &str) -> Value {
    let (s, body) = get(router, &format!("/api/v1/insights/{name}?{query}")).await;
    assert_eq!(s, 200, "{name}?{query}: {body}");
    serde_json::from_str::<Value>(&body).expect("json")["data"].clone()
}

fn num(v: &Value) -> String {
    v.as_u64().map(|n| n.to_string()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn api_and_pages_give_the_same_numbers_for_the_same_filter() {
    let root = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(&root.path().join("repo"))
        .at("2024-03-01T12:00:00Z")
        .write("src/a.rs", "fn a() {}\n")
        .write("Cargo.lock", "lock\n")
        .commit("feat(core): first")
        .at("2024-03-20T12:00:00Z")
        .write("src/a.rs", "fn a() {}\nfn b() {}\n")
        .write("src/b.rs", "x\ny\n")
        .write("Cargo.lock", "lock\nmore\n")
        .commit("fix(core): second")
        .at("2024-04-10T12:00:00Z")
        .write("docs/x.md", "doc\n")
        .write("src/b.rs", "x\n")
        .commit("docs: third");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    gitterrace_testkit::wait::page_until(&router, "/stats", gitterrace_testkit::wait::indexed)
        .await;

    // Each query narrows differently: none, a type, a path, a period, and options that change
    // which files count. The pages read them the same way the API does.
    for q in [
        "",
        "q=type%3Afix",
        "q=path%3Asrc",
        "period=1m",
        "generated=1&exclude=docs/*",
        "q=author%3At%40e.x&period=3m",
    ] {
        let sum = api(&router, "summary", q).await;
        if sum["current"]["commits"].as_u64().is_none_or(|n| n == 0) {
            continue;
        }

        let (_, page) = get(&router, &format!("/stats/hotspots?{q}")).await;
        let web: Vec<(String, String)> = rows(&page, "data-act=\"insight\"")
            .into_iter()
            .map(|r| (r[0].clone(), r[3].clone()))
            .collect();
        let hot = api(&router, "hotspots", &format!("{q}&limit=50")).await;
        let api_rows: Vec<(String, String)> = hot["files"]
            .as_array()
            .expect("files")
            .iter()
            .map(|h| (h["path"].as_str().expect("path").to_owned(), num(&h["changes"])))
            .collect();
        assert_eq!(web, api_rows, "Hotspots page against API for {q:?}");

        let (_, page) = get(&router, &format!("/stats/contributors?{q}")).await;
        let people = api(&router, "contributors", q).await;
        let web: Vec<String> =
            rows(&page, "class=\"av ").into_iter().map(|r| r[2].clone()).collect();
        let api_rows: Vec<String> = people["contributors"]
            .as_array()
            .expect("contributors")
            .iter()
            .filter(|p| p["bot"] != true)
            // The page renders the top rows and links to the rest; the API lists every one.
            .take(100)
            .map(|p| num(&p["commits"]))
            .collect();
        assert_eq!(web, api_rows, "Contributors page against API for {q:?}");
    }
}
