use gitterrace_config::{Config, parse};
use gitterrace_testkit::git;
use rmcp::model::RawContent;

use super::McpServer;
use crate::{ApiError, ApiState};

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
        .repo_overview(rmcp::handler::server::wrapper::Parameters(
            super::RepoArgs::default(),
        ))
        .await
        .expect("a result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "{}", text(&res));
    assert!(text(&res).contains("switched off"), "{}", text(&res));

    let (_d, on) = server("");
    let res = on
        .repo_overview(rmcp::handler::server::wrapper::Parameters(
            super::RepoArgs::default(),
        ))
        .await
        .expect("result");
    assert_ne!(res.is_error, Some(true), "{}", text(&res));
}

fn fixture(root: &std::path::Path, name: &str, msg: &str) -> std::path::PathBuf {
    let p = root.join(name);
    std::fs::create_dir(&p).expect("mkdir");
    git(&p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), format!("{name}\n")).expect("write");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", msg]);
    p
}

fn entry(name: &str, path: std::path::PathBuf) -> super::McpRepo {
    super::McpRepo {
        name: name.to_owned(),
        path,
        unavailable: None,
    }
}

/// Two repositories, the second with blame off, and one recorded folder that is gone.
fn two(config: &str) -> (tempfile::TempDir, McpServer) {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = fixture(dir.path(), "alpha", "feat: alpha one");
    let b = fixture(dir.path(), "beta", "feat: beta one");
    let gone = super::McpRepo {
        name: "gone".to_owned(),
        path: dir.path().join("gone"),
        unavailable: Some("the folder is gone".to_owned()),
    };
    let cfg: Config = parse(config).expect("parse").config;
    let list = vec![entry("alpha", a), entry("beta", b), gone];
    (dir, McpServer::over("test scope", list).with_config(&cfg))
}

#[tokio::test]
async fn list_repos_names_every_repository_in_scope_with_its_last_commit() {
    let (_d, s) = two("");
    let res = s.list_repos().await.expect("result");
    let v: serde_json::Value = serde_json::from_str(&text(&res)).expect("json");
    assert_eq!(v["scope"], "test scope");
    let repos = v["repos"].as_array().expect("repos");
    assert_eq!(repos.len(), 3);
    assert_eq!(repos[0]["name"], "alpha");
    assert!(
        repos[0].get("path").is_none(),
        "a host path would reach the model"
    );
    assert_eq!(repos[0]["last_commit"]["subject"], "feat: alpha one");
    assert_eq!(repos[1]["head_branch"], "main");
    assert_eq!(repos[2]["available"], false);
    assert_eq!(repos[2]["error"], "the folder is gone");
}

#[tokio::test]
async fn a_tool_reads_the_repository_its_repo_argument_names() {
    let (_d, s) = two("");
    let args = |repo: &str| {
        rmcp::handler::server::wrapper::Parameters(super::LogArgs {
            repo: Some(repo.to_owned()),
            rev: None,
            path: None,
            page: None,
            per_page: None,
        })
    };
    let res = s.log(args("beta")).await.expect("result");
    assert!(text(&res).contains("beta one"), "{}", text(&res));
    assert!(!text(&res).contains("alpha one"), "{}", text(&res));
}

#[tokio::test]
async fn a_missing_or_unknown_repo_is_refused_with_the_names_in_scope() {
    let (_d, s) = two("");
    let call = |repo: Option<&str>| {
        rmcp::handler::server::wrapper::Parameters(super::RepoArgs {
            repo: repo.map(str::to_owned),
        })
    };
    for (repo, want) in [
        (None, "pass repo"),
        (Some("../alpha"), "unknown repo"),
        (Some("gone"), "unavailable"),
    ] {
        let res = s.repo_overview(call(repo)).await.expect("result");
        assert_eq!(res.is_error, Some(true), "{}", text(&res));
        assert!(text(&res).contains(want), "{repo:?}: {}", text(&res));
    }
}

#[tokio::test]
async fn a_feature_off_for_one_repository_refuses_only_there() {
    let (_d, s) = two("[repo.beta.features]\nblame_enabled = false\n");
    let blame = |repo: &str| {
        rmcp::handler::server::wrapper::Parameters(super::BlameArgs {
            repo: Some(repo.to_owned()),
            rev: None,
            path: "a.txt".to_owned(),
        })
    };
    let off = s.blame(blame("beta")).await.expect("result");
    assert!(text(&off).contains("switched off"), "{}", text(&off));
    let on = s.blame(blame("alpha")).await.expect("result");
    assert_ne!(on.is_error, Some(true), "{}", text(&on));
}

#[tokio::test]
async fn list_repos_leaks_no_path_head_or_subject_of_a_repo_with_api_off() {
    // The parser refuses api_enabled under [repo.NAME.features], so the override is set directly.
    let (_d, s) = two("");
    let mut cfg = Config::default();
    cfg.repo.insert(
        "beta".to_owned(),
        gitterrace_config::RepoOverride {
            features_off: vec![gitterrace_config::Feature::Api],
            ..Default::default()
        },
    );
    let s = s.with_config(&cfg);
    let res = s.list_repos().await.expect("result");
    let raw = text(&res);
    let v: serde_json::Value = serde_json::from_str(&raw).expect("json");
    let beta = &v["repos"][1];
    assert_eq!(beta["name"], "beta");
    assert_eq!(beta["error"], "api off");
    assert_eq!(beta["available"], false);
    assert!(beta["head"].is_null() && beta["last_commit"].is_null());
    assert!(!raw.contains("beta one"), "{raw}");
    assert!(!raw.contains("/beta"), "{raw}");
    assert!(raw.contains("alpha one"), "{raw}");
}

#[test]
fn concurrent_first_calls_on_a_cold_repo_open_it_once() {
    let (_d, s) = two("");
    let entry = s.repos.pick(Some("alpha")).expect("alpha").clone();
    let cfg = Config::default();
    std::thread::scope(|sc| {
        for _ in 0..8 {
            sc.spawn(|| s.repos.state(&entry, &cfg).expect("state"));
        }
    });
    assert_eq!(s.repos.opens(), 1);
}

/// The `query` argument and the older keys say one thing: the same JSON either way, for both
/// tools that take it. A key that folded differently from its keyword would split them.
#[tokio::test]
async fn query_and_the_older_keys_give_equal_json() {
    use rmcp::handler::server::wrapper::Parameters;
    let (_d, s) = server("");
    let hot = |period: Option<&str>, query: Option<&str>| {
        Parameters(super::HotspotArgs {
            repo: None,
            rev: None,
            period: period.map(str::to_owned),
            path: None,
            query: query.map(str::to_owned),
            limit: None,
        })
    };
    let act = |period: Option<&str>, query: Option<&str>| {
        Parameters(super::ActivityArgs {
            repo: None,
            rev: None,
            period: period.map(str::to_owned),
            query: query.map(str::to_owned),
        })
    };
    let (a, b) = (
        s.hotspots(hot(Some("1m"), None)).await.expect("a"),
        s.hotspots(hot(None, Some("after:1m"))).await.expect("b"),
    );
    assert_ne!(a.is_error, Some(true), "{}", text(&a));
    assert_eq!(text(&a), text(&b));
    let (a, b) = (
        s.activity(act(Some("1m"), None)).await.expect("a"),
        s.activity(act(None, Some("after:1m"))).await.expect("b"),
    );
    assert_ne!(a.is_error, Some(true), "{}", text(&a));
    assert_eq!(text(&a), text(&b));
    // A bad query is a readable error naming the span, not a panic.
    let bad = s.activity(act(None, Some("nope:x"))).await.expect("bad");
    assert_eq!(bad.is_error, Some(true), "{}", text(&bad));
}

#[test]
fn mcp_tool_error_emits_server_error_message_for_5xx() {
    let inner = ApiError::Task("/home/me/.git/gitterrace/x: denied".to_owned());
    assert_eq!(super::tool_error_text(&inner), crate::SERVER_ERROR_MESSAGE);
    let client = ApiError::BadRequest("bad rev".to_owned());
    assert_eq!(super::tool_error_text(&client), client.to_string());
}
