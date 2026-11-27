use gitterrace_config::{Config, parse};
use gitterrace_testkit::git;

use super::McpServer;
use super::tests::{entry, fixture, text};

/// Two repositories, `alpha` holding a ten line file, and the permalink payload v1 would carry
/// for its lines 3 to 5, built by the same function the payload uses.
fn linked(config: &str) -> (tempfile::TempDir, McpServer, String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = fixture(dir.path(), "alpha", "feat: alpha one");
    let b = fixture(dir.path(), "beta", "feat: beta one");
    let body: String = (1..=10).map(|n| ["line ", &n.to_string(), "\n"].concat()).collect();
    std::fs::write(a.join("a.txt"), body).expect("write");
    git(&a, &["add", "."]);
    git(&a, &["commit", "-q", "-m", "feat: ten lines"]);
    let id =
        |p: &std::path::Path| gitterrace_git::Repo::discover(p).expect("repo").id().expect("id");
    let (alpha, beta) = (id(&a), id(&b));
    let sha = gitterrace_git::Repo::discover(&a).expect("repo").resolve("HEAD").expect("head");
    let lines = gitterrace_actions::Lines { start: 3, end: 5 };
    let link = gitterrace_actions::styled(
        gitterrace_types::LinkStyle::Gitterrace,
        "",
        Some(&alpha),
        "alpha",
        &sha,
        Some("a.txt"),
        Some(lines),
    );
    let cfg: Config = parse(config).expect("parse").config;
    let list = vec![entry("alpha", a), entry("beta", b)];
    let server = McpServer::over("test scope", list).with_config(&cfg);
    (dir, server, link, beta)
}

async fn resolve(
    s: &McpServer,
    url: &str,
    repo: Option<&str>,
    ctx: Option<usize>,
) -> rmcp::model::CallToolResult {
    let a = super::PermalinkArgs {
        repo: repo.map(str::to_owned),
        url: url.to_owned(),
        context_lines: ctx,
    };
    s.resolve_permalink(rmcp::handler::server::wrapper::Parameters(a)).await.expect("result")
}

#[tokio::test]
async fn a_payload_permalink_resolves_to_the_lines_it_names() {
    let (_d, s, link, _) = linked("");
    let res = resolve(&s, &link, None, None).await;
    assert_ne!(res.is_error, Some(true), "{}", text(&res));
    let v: serde_json::Value = serde_json::from_str(&text(&res)).expect("json");
    assert_eq!(v["text"], "line 3\nline 4\nline 5");
    assert_eq!(v["path"], "a.txt");
    assert_eq!(v["lines"], serde_json::json!({ "start": 3, "end": 5 }));
    assert_eq!(v["truncated"], false);
    assert_eq!(v["anchor"]["status"], "untouched");
    let ctx = resolve(&s, &link, Some("alpha"), Some(1)).await;
    let v: serde_json::Value = serde_json::from_str(&text(&ctx)).expect("json");
    assert_eq!(v["text"], "line 2\nline 3\nline 4\nline 5\nline 6");
}

#[tokio::test]
async fn a_link_to_another_repository_is_refused() {
    let (_d, s, link, beta) = linked("");
    let foreign = link.replacen(&link["gitterrace://".len()..][..12], &beta, 1);
    // Beta is in scope but has no such commit: nothing of alpha is read through it.
    let res = resolve(&s, &foreign, None, None).await;
    assert_eq!(res.is_error, Some(true), "{}", text(&res));
    assert!(!text(&res).contains("line 3"), "{}", text(&res));
    // Naming alpha for a link that carries beta's id is refused outright.
    let res = resolve(&s, &foreign, Some("alpha"), None).await;
    assert_eq!(res.is_error, Some(true), "{}", text(&res));
    assert!(text(&res).contains("not alpha"), "{}", text(&res));
    // An id in no repository in scope.
    let unknown = link.replacen(&link["gitterrace://".len()..][..12], "000000000000", 1);
    let res = resolve(&s, &unknown, None, None).await;
    assert!(text(&res).contains("not in scope"), "{}", text(&res));
}

#[tokio::test]
async fn a_link_that_climbs_out_of_the_base_is_refused() {
    let (_d, s, link, _) = linked("");
    let up = link.replace("/a.txt", "/../../etc/passwd");
    let encoded = link.replace("/a.txt", "/%2e%2e/a.txt");
    let host = link.replace("/a.txt", "//etc/passwd");
    for bad in [up, encoded, host, "/etc/passwd".to_owned(), "file:///etc/passwd".to_owned()] {
        let res = resolve(&s, &bad, None, None).await;
        assert_eq!(res.is_error, Some(true), "{bad}: {}", text(&res));
        assert!(text(&res).contains("not a gitterrace permalink"), "{bad}: {}", text(&res));
    }
    let res = resolve(&s, &link, None, Some(51)).await;
    assert_eq!(res.is_error, Some(true), "{}", text(&res));
}

#[tokio::test]
async fn a_range_over_the_blob_cap_is_cut_and_says_so() {
    let (_d, s, link, _) = linked("[api]\nblob_lines = 2\n");
    let res = resolve(&s, &link, None, None).await;
    let v: serde_json::Value = serde_json::from_str(&text(&res)).expect("json");
    assert_eq!(v["text"], "line 3\nline 4");
    assert_eq!(v["lines"], serde_json::json!({ "start": 3, "end": 4 }));
    assert_eq!(v["truncated"], true);
}

#[tokio::test]
async fn the_file_resource_reads_the_same_text_and_a_foreign_one_is_refused() {
    let (_d, s, link, _) = linked("");
    let got = s.resource_read(&link).await.expect("read");
    let rmcp::model::ResourceContents::TextResourceContents { text, .. } = &got.contents[0] else {
        panic!("a text resource");
    };
    assert_eq!(text, "line 3\nline 4\nline 5");
    let gone = link.replacen(&link["gitterrace://".len()..][..12], "000000000000", 1);
    assert!(s.resource_read(&gone).await.is_err());
    assert!(s.resource_read("gitterrace://x/../y").await.is_err());
    assert!(s.resource_read("file:///etc/passwd").await.is_err());
    let list = s.resource_list().await;
    assert_eq!(list.resources.len(), 4, "an overview and a basket per repository");
    assert_eq!(McpServer::resource_templates().resource_templates.len(), 1);
}

#[tokio::test]
async fn a_public_server_lists_no_basket_resource_and_refuses_to_read_one() {
    // The API is asked for, so the overview stays: only the local-only basket goes.
    let (_d, s, link, _) = linked("[server]\nis_public = true\n[features]\napi_enabled = true\n");
    let list = s.resource_list().await;
    assert_eq!(list.resources.len(), 2, "an overview per repository and no basket");
    let id = &link["gitterrace://".len()..][..12];
    let basket = format!("gitterrace://{id}/findings");
    assert!(s.resource_read(&basket).await.is_err(), "the basket is local only");
}
