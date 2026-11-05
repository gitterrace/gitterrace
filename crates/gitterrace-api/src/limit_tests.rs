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
    assert_eq!(
        count(&json(&s.log(None, None, 1, 100).expect("log")), "commits"),
        10
    );
    let s = with(s, |c| c.api.log_page_commits = 3);
    assert_eq!(
        count(&json(&s.log(None, None, 1, 100).expect("log")), "commits"),
        3
    );
}

#[test]
fn log_skip_commits_refuses_deeper_pages() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    assert!(s.log(None, None, 3, 2).is_ok());
    let s = with(s, |c| c.api.log_skip_commits = 3);
    let err = s.log(None, None, 3, 2).expect_err("skip 4 > 3");
    assert!(err.to_string().contains("api.log_skip_commits"), "{err}");
}

#[test]
fn blob_lines_and_blob_kb_cut_the_text() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = state(dir.path());
    let text = |s: &ApiState| {
        let v = json(&s.blob(None, "a.txt", None, None).expect("blob"));
        (
            v["text"].as_str().unwrap_or_default().to_owned(),
            v["truncated"] == true,
        )
    };
    assert_eq!(text(&s).0.lines().count(), 10);
    let s = with(s, |c| c.api.blob_lines = 4);
    assert_eq!(
        text(&s),
        ("line 0\nline 1\nline 2\nline 3".to_owned(), true)
    );
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
    assert_eq!(
        count(&json(&s.blame(None, "a.txt").expect("blame")), "lines"),
        10
    );
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
            hs.iter()
                .map(|h| h["lines"].as_array().map_or(0, Vec::len))
                .sum::<usize>()
        })
    };
    assert!(lines(&s) > 1);
    let s = with(s, |c| c.api.diff_lines = 1);
    assert_eq!(lines(&s), 1);
}
