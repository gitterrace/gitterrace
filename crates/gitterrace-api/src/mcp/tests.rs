use gitterrace_config::{Config, parse};
use gitterrace_testkit::git;
use rmcp::model::RawContent;

use super::McpServer;
use crate::ApiState;

fn server(config: &str) -> (tempfile::TempDir, McpServer) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat(core): first"]);
    let state = ApiState::open(p).expect("open");
    let cfg: Config = parse(config).expect("parse").config;
    (dir, McpServer::new(state).with_config(&cfg))
}

fn text(res: &rmcp::model::CallToolResult) -> String {
    res.content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

/// The refusal is in the call path, not only in the table `tool_on` reads: a tool the config
/// switches off answers with an error and never runs, one left on answers with its data.
#[tokio::test]
async fn a_called_tool_is_refused_when_its_switch_is_off() {
    let (_d, off) = server("[features]\napi_enabled = false\n");
    let res = off
        .repo_overview()
        .await
        .expect("a result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "{}", text(&res));
    assert!(text(&res).contains("switched off"), "{}", text(&res));

    let (_d, on) = server("");
    let res = on.repo_overview().await.expect("result");
    assert_ne!(res.is_error, Some(true), "{}", text(&res));
}
