use super::McpServer;
use super::tests::{call, server, text};
use crate::ApiState;

/// The continue prompt is listed, names the tools it sends the agent to and quotes notes as data.
#[test]
fn the_continue_prompt_names_work_state_and_marks_notes_untrusted() {
    let listed = McpServer::prompt_router().list_all();
    assert!(listed.iter().any(|p| p.name == "continue_work"));
    let text = crate::mcp::prompts::continue_text(&crate::mcp::prompts::ContinueArgs {
        repo: Some("al`pha".into()),
        since: Some("2h".into()),
    });
    for part in ["work_state", "worktree_diff", "resolve_permalink", "untrusted", "`alpha`", "`2h`"]
    {
        assert!(text.contains(part), "{part}");
    }
}

/// `work_state` lists the dirty file, the checkpoint delta, the finding and the commits, and the
/// tool is declared read-only.
#[tokio::test]
async fn work_state_lists_the_dirty_tree_the_checkpoint_the_finding_and_the_commits() {
    use gitterrace_types::api::NewFinding;

    let (d, s) = server("");
    let state = ApiState::open(d.path()).expect("open");
    state.checkpoint_create("start", false).expect("checkpoint");
    std::fs::write(d.path().join("b.txt"), "one\ntwo\n").expect("write");
    let new = NewFinding {
        kind: "file".into(),
        lines: None,
        note: "check b".into(),
        path: Some("a.txt".into()),
        query: None,
        rev: "HEAD".into(),
        sha: None,
    };
    state.finding_add(&new).expect("add");
    let args: super::WorkStateArgs = serde_json::from_value(serde_json::json!({})).expect("args");
    let v = call(s.work_state(rmcp::handler::server::wrapper::Parameters(args)).await);
    assert!(v["head"].is_string());
    assert_eq!(v["branch"], "main");
    assert_eq!(v["recent"].as_array().map(Vec::len), Some(1));
    let files = v["since"]["files"].as_array().expect("files");
    let paths: Vec<&str> = files.iter().filter_map(|f| f["path"].as_str()).collect();
    assert!(paths.contains(&"b.txt"), "{paths:?}");
    assert_eq!(v["worktree"]["files_omitted"], false);
    assert_eq!(v["findings_total"], 1);
    assert_eq!(v["findings"][0]["note"], "check b");
    assert_eq!(v["findings"][0]["n"], 1);
    assert!(v["next"].as_array().is_some_and(|n| !n.is_empty()));
    let tool = McpServer::tool_docs().into_iter().find(|t| t.name == "work_state").expect("tool");
    assert!(tool.read_only && tool.description.contains("untrusted"));
}

/// A repository with nothing uncommitted and no checkpoint answers empty lists, not an error.
#[tokio::test]
async fn work_state_of_a_clean_repository_has_empty_sections() {
    let (_d, s) = server("");
    let args: super::WorkStateArgs = serde_json::from_value(serde_json::json!({})).expect("args");
    let v = call(s.work_state(rmcp::handler::server::wrapper::Parameters(args)).await);
    assert_eq!(v["worktree"]["files_total"], 0);
    assert!(v["since"].is_null());
    assert_eq!(v["findings_total"], 0);
}

/// The tool is refused with the working tree switch off, like the reads it composes.
#[tokio::test]
async fn work_state_is_refused_when_the_working_tree_is_off() {
    let (_d, s) = server("[features]\nworktree_enabled = false\n");
    let args: super::WorkStateArgs = serde_json::from_value(serde_json::json!({})).expect("args");
    let res = s.work_state(rmcp::handler::server::wrapper::Parameters(args)).await.expect("result");
    assert_eq!(res.is_error, Some(true), "{}", text(&res));
}

/// A public handle refuses the tool and the answer holds nothing of the working tree.
#[tokio::test]
async fn work_state_is_refused_on_a_public_server() {
    let (d, _) = server("");
    std::fs::write(d.path().join("a.txt"), "secret\n").expect("write");
    let public = McpServer::new(ApiState::open(d.path()).expect("open").committed_only());
    let args: super::WorkStateArgs = serde_json::from_value(serde_json::json!({})).expect("args");
    let res = public.work_state(rmcp::handler::server::wrapper::Parameters(args)).await.expect("r");
    assert_eq!(res.is_error, Some(true), "{}", text(&res));
    assert!(!text(&res).contains("secret"));
}
