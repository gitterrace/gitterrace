//! Guard class: every insight in the registry answers the same bytes over HTTP, MCP and the
//! command line. An entry that one surface cannot serve, or serves differently, fails here.

#![expect(clippy::expect_used, reason = "test setup")]

use std::io::Write as _;
use std::process::{Command, Stdio};

use gitterrace_api::{ApiState, INSIGHTS};
use gitterrace_testkit::Fixture;

const BIN: &str = env!("CARGO_BIN_EXE_git-terrace");

fn cli(repo: &std::path::Path, cfg: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(BIN)
        .arg("--config")
        .arg(cfg)
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git-terrace");
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8").trim_end().to_owned()
}

/// Every `insight` call answered by one `git-terrace mcp` over stdio, by request id.
fn mcp(repo: &std::path::Path, cfg: &std::path::Path) -> Vec<String> {
    let line = |v: serde_json::Value| format!("{v}\n");
    let mut ask = line(serde_json::json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                   "clientInfo": {"name": "t", "version": "1"}}}));
    ask += &line(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    for (i, ins) in INSIGHTS.iter().enumerate() {
        ask += &line(serde_json::json!({"jsonrpc": "2.0", "id": i + 1, "method": "tools/call",
            "params": {"name": "insight", "arguments": {"name": ins.name}}}));
    }
    let mut child = Command::new(BIN)
        .arg("--config")
        .arg(cfg)
        .arg("mcp")
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mcp");
    child.stdin.take().expect("stdin").write_all(ask.as_bytes()).expect("write");
    let out = child.wait_with_output().expect("mcp output");
    let mut texts = vec![String::new(); INSIGHTS.len()];
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        let v: serde_json::Value = serde_json::from_str(l).expect("json-rpc line");
        let Some(id) = v["id"].as_u64().filter(|i| *i > 0) else { continue };
        assert_ne!(v["result"]["isError"], true, "{v}");
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or_default();
        text.clone_into(&mut texts[id as usize - 1]);
    }
    texts
}

#[tokio::test(flavor = "multi_thread")]
async fn every_insight_answers_the_same_bytes_on_http_mcp_and_the_command_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).expect("mkdir");
    let _ = Fixture::standard(&repo);
    let cfg = dir.path().join("cfg");
    std::fs::create_dir(&cfg).expect("mkdir");

    let state = ApiState::open(&repo).expect("open");
    state.refresh_index().expect("index");
    let router = gitterrace_api::router(state);
    let over_mcp = mcp(&repo, &cfg);
    for (ins, mcp) in INSIGHTS.iter().zip(&over_mcp) {
        let (status, http) =
            gitterrace_testkit::get(&router, &format!("/insights/{}", ins.name)).await;
        assert_eq!(status, 200, "{}: {http}", ins.name);
        let cli = cli(&repo, &cfg, &["insights", ins.name, "--json"]);
        assert_eq!(http, cli, "{}: HTTP and CLI differ", ins.name);
        assert_eq!(&http, mcp, "{}: HTTP and MCP differ", ins.name);
    }
    let listed = cli(&repo, &cfg, &["insights", "--json"]);
    let (_, http) = gitterrace_testkit::get(&router, "/insights").await;
    assert_eq!(http, listed, "the insight list");
}
