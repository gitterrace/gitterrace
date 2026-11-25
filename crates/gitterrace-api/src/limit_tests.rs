//! Each `[api]` setting reaches its answer: a smaller value caps what one answer carries.

use gitterrace_config::Config;
use gitterrace_testkit::git;

use crate::ApiState;

/// Ten commits, each adding one line to `a.txt`.
fn state(dir: &std::path::Path) -> ApiState {
    git(dir, &["init", "-q", "-b", "main"]);
    let mut text = String::new();
    for i in 0..10 {
        text += "line ";
        text += &i.to_string();
        text.push('\n');
        std::fs::write(dir.join("a.txt"), &text).expect("write");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", &format!("c{i}")]);
    }
    ApiState::open(dir).expect("open")
}

fn with(s: ApiState, f: impl FnOnce(&mut Config)) -> ApiState {
    let mut c = Config::default();
    f(&mut c);
    c.validate().expect("valid");
    s.configured(&c)
}

fn json(v: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(v).expect("json")
}

fn count(v: &serde_json::Value, key: &str) -> usize {
    v[key].as_array().map_or(0, Vec::len)
}

#[test]
fn log_page_commits_caps_the_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    assert_eq!(count(&json(&s.log(None, None, Some(1), Some(100)).expect("log")), "commits"), 10);
    let s = with(s, |c| c.api.log_page_commits = 3);
    assert_eq!(count(&json(&s.log(None, None, Some(1), Some(100)).expect("log")), "commits"), 3);
}

#[test]
fn log_skip_commits_refuses_deeper_pages() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    assert!(s.log(None, None, Some(3), Some(2)).is_ok());
    let s = with(s, |c| c.api.log_skip_commits = 3);
    let err = s.log(None, None, Some(3), Some(2)).expect_err("skip 4 > 3");
    assert!(err.to_string().contains("api.log_skip_commits"), "{err}");
}

#[test]
fn blob_lines_and_blob_kb_cut_the_text() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    let text = |s: &ApiState| {
        let v = json(&s.blob(None, "a.txt", None, None).expect("blob"));
        (v["text"].as_str().unwrap_or_default().to_owned(), v["truncated"] == true)
    };
    assert_eq!(text(&s).0.lines().count(), 10);
    let s = with(s, |c| c.api.blob_lines = 4);
    assert_eq!(text(&s), ("line 0\nline 1\nline 2\nline 3".to_owned(), true));
}

#[test]
fn blob_kb_cuts_by_bytes() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    std::fs::write(dir.path().join("wide.txt"), "y".repeat(3000)).expect("write");
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "wide"]);
    let len = |s: &ApiState| {
        let v = json(&s.blob(None, "wide.txt", None, None).expect("blob"));
        v["text"].as_str().map_or(0, str::len)
    };
    assert_eq!(len(&s), 3000);
    let s = with(s, |c| c.api.blob_kb = 1);
    assert_eq!(len(&s), 1024);
}

#[test]
fn blame_lines_cut_the_blame() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    assert_eq!(count(&json(&s.blame(None, "a.txt").expect("blame")), "lines"), 10);
    let s = with(s, |c| c.api.blame_lines = 2);
    let v = json(&s.blame(None, "a.txt").expect("blame"));
    assert_eq!((count(&v, "lines"), v["truncated"] == true), (2, true));
}

#[test]
fn diff_lines_cut_the_commit_diff() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    let lines = |s: &ApiState| {
        let v = json(&s.commit("HEAD", true, None).expect("commit"));
        v["files"][0]["hunks"].as_array().map_or(0, |hs| {
            hs.iter().map(|h| h["lines"].as_array().map_or(0, Vec::len)).sum::<usize>()
        })
    };
    assert!(lines(&s) > 1);
    let s = with(s, |c| c.api.diff_lines = 1);
    assert_eq!(lines(&s), 1);
}

/// Guard class: a file list the scan cut must say so on every transport that serves the working
/// tree, with the diff-line flag left alone. The HTTP route and the MCP tool call the same
/// core methods, so one body check covers the wire shape and the core answers cover both.
#[tokio::test]
async fn a_cut_file_list_is_flagged_on_the_worktree_the_plan_and_the_checkpoint() {
    let (d, s) = crate::tests::fixture();
    for i in 0..4 {
        std::fs::write(d.path().join(format!("n{i}.txt")), "x\n").expect("write");
    }
    let omitted = |v: &serde_json::Value| v["files_omitted"].as_bool();
    s.repo.set_worktree_limits(50, 1 << 20);
    let (_, whole) = crate::tests::get(&s, "/worktree").await;
    assert_eq!((omitted(&whole), whole["truncated"].as_bool()), (Some(false), Some(false)));
    let (id, _) = s.checkpoint_create("", false).expect("checkpoint");
    assert!(!s.checkpoints().expect("list").checkpoints[0].files_omitted);
    let plan = s.review_plan(&crate::PlanRequest::default(), &crate::insights::Params::default());
    assert!(!plan.expect("plan").files_omitted);

    s.repo.set_worktree_limits(2, 1 << 20);
    let (_, cut) = crate::tests::get(&s, "/worktree").await;
    assert_eq!((omitted(&cut), cut["truncated"].as_bool()), (Some(true), Some(false)));
    let core = s.worktree_diff(None, None).expect("core");
    assert!(core.files_omitted && !core.truncated, "the MCP tool serves this answer");
    let (_, since) = crate::tests::get(&s, &format!("/worktree?since={id}")).await;
    assert_eq!(omitted(&since), Some(true));
    assert!(
        s.review_plan(&crate::PlanRequest::default(), &crate::insights::Params::default())
            .expect("plan")
            .files_omitted
    );
    let (cut_id, _) = s.checkpoint_create("cut", false).expect("checkpoint");
    let items = s.checkpoints().expect("list").checkpoints;
    let by = |id: &str| items.iter().find(|c| c.id == id).map(|c| c.files_omitted);
    assert_eq!(by(&cut_id), Some(true), "a checkpoint of a cut scan says so");
    assert_eq!(by(&id), Some(false));
}
