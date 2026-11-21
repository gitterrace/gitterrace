//! The Languages page and the API's `languages` insight give one size.
//!
//! The page's "now" table counts the first-parent history, the insight counts the tree at the
//! revision. They are the project's one size metric, so the total and each language's lines
//! must be equal.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::matrix::Mailmap;
use gitterrace_testkit::{oracle, page};

/// The "Languages now" table as `(name, lines)` rows, `Total` and `Other` included.
fn table(html: &str) -> Vec<(String, u64)> {
    let start = html.find("aria-label=\"Languages now\"").expect("the now table");
    let end = html[start..].find("</table>").map_or(html.len(), |e| start + e);
    html[start..end]
        .split("<tr>")
        .skip(1)
        .filter_map(|tr| {
            let name = tr.split("</th>").next()?.rsplit('>').next()?.trim().to_owned();
            let lines = tr.split("<td class=\"r\">").nth(1)?.split('<').next()?;
            Some((name, lines.replace(',', "").parse().ok()?))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_and_the_api_count_the_same_lines() {
    let dir = tempfile::tempdir().expect("tmp");
    let (repo, _) = oracle::matrix_repo(dir.path(), Mailmap::AsIs);
    let router = crate::common::ready_router(&repo).await;

    let (status, html) = page(&router, "/stats/languages").await;
    assert_eq!(status, 200, "{html}");
    let rows = table(&html);
    let total: u64 = rows.iter().filter(|(n, _)| n != "Total").map(|(_, l)| l).sum();
    let (status, body) = page(&router, "/api/v1/insights/languages").await;
    assert_eq!(status, 200, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    let data = &json["data"];

    assert!(total > 0, "the page shows a total: {html}");
    assert_eq!(data["total_lines"], total, "the total of the page and of the API");
    let sum: u64 = data["languages"]
        .as_array()
        .expect("languages")
        .iter()
        .map(|l| l["lines"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(sum, total, "the API's languages add up to its total");
    for (name, lines) in rows.into_iter().filter(|(n, _)| n != "Other" && n != "Total") {
        let api = data["languages"]
            .as_array()
            .expect("languages")
            .iter()
            .find(|l| l["language"] == name.as_str())
            .unwrap_or_else(|| panic!("the API lists {name}"));
        assert_eq!(api["lines"], lines, "{name}");
    }
}
