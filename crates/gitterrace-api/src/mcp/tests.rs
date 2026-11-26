use gitterrace_config::{Config, parse};
use gitterrace_testkit::{Fixture, git};

use super::McpServer;
use crate::{ApiError, ApiState};

fn server(config: &str) -> (tempfile::TempDir, McpServer) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let _ = Fixture::init(p).write("a.txt", "one\n").commit("feat(core): first");
    let state = ApiState::open(p).expect("open");
    let cfg: Config = parse(config).expect("parse").config;
    (dir, McpServer::new(state).with_config(&cfg))
}

fn text(res: &rmcp::model::CallToolResult) -> String {
    res.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect()
}

/// The refusal is in the call path, not only in the table `tool_on` reads: a tool the config
/// switches off answers with an error and never runs, one left on answers with its data.
#[tokio::test]
async fn a_called_tool_is_refused_when_its_switch_is_off() {
    let (_d, off) = server("[features]\napi_enabled = false\n");
    let res = off
        .repo_overview(rmcp::handler::server::wrapper::Parameters(super::RepoArgs::default()))
        .await
        .expect("a result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "{}", text(&res));
    assert!(text(&res).contains("switched off"), "{}", text(&res));

    let (_d, on) = server("");
    let res = on
        .repo_overview(rmcp::handler::server::wrapper::Parameters(super::RepoArgs::default()))
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
    super::McpRepo { name: name.to_owned(), path, unavailable: None }
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
    assert!(repos[0].get("path").is_none(), "a host path would reach the model");
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
    // Guard class: a default the HTTP route and the MCP tool each spell out drifts apart.
    let per_page = format!("\"per_page\":{}", crate::core::DEFAULT_PER_PAGE);
    assert!(text(&res).contains(&per_page), "{}", text(&res));
}

#[tokio::test]
async fn a_missing_or_unknown_repo_is_refused_with_the_names_in_scope() {
    let (_d, s) = two("");
    let call = |repo: Option<&str>| {
        rmcp::handler::server::wrapper::Parameters(super::RepoArgs {
            repo: repo.map(str::to_owned),
        })
    };
    for (repo, want) in
        [(None, "pass repo"), (Some("../alpha"), "unknown repo"), (Some("gone"), "unavailable")]
    {
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

/// The `query` argument and `period` say one thing: the same JSON either way, for every
/// insight. A key that folded differently from its keyword would split them.
#[tokio::test]
async fn query_and_period_give_equal_json_for_every_insight() {
    use rmcp::handler::server::wrapper::Parameters;
    let (_d, s) = server("");
    let ask = |name: &str, period: Option<&str>, query: Option<&str>| {
        Parameters(super::InsightArgs {
            name: name.to_owned(),
            period: period.map(str::to_owned),
            query: query.map(str::to_owned),
            ..super::InsightArgs::default()
        })
    };
    for ins in crate::INSIGHTS {
        let (a, b) = (
            s.insight(ask(ins.name, Some("1m"), None)).await.expect("a"),
            s.insight(ask(ins.name, None, Some("after:1m"))).await.expect("b"),
        );
        assert_ne!(a.is_error, Some(true), "{}: {}", ins.name, text(&a));
        assert_eq!(text(&a), text(&b), "{}", ins.name);
    }
    // A bad query or name is a readable error, not a panic.
    for (name, q) in [("activity", Some("nope:x")), ("nope", None)] {
        let bad = s.insight(ask(name, None, q)).await.expect("bad");
        assert_eq!(bad.is_error, Some(true), "{}", text(&bad));
    }
}

#[test]
fn mcp_tool_error_emits_server_error_message_for_5xx() {
    let inner = ApiError::Task("/home/me/.git/gitterrace/x: denied".to_owned());
    assert_eq!(super::tool_error_text(&inner), crate::SERVER_ERROR_MESSAGE);
    let client = ApiError::BadRequest("bad rev".to_owned());
    assert_eq!(super::tool_error_text(&client), client.to_string());
}

/// A tool argument the schema does not list is refused, never dropped: an old `path` key would
/// otherwise answer unfiltered numbers.
#[test]
fn unknown_tool_arguments_are_refused() {
    let bad = serde_json::json!({ "name": "hotspots", "type": "fix" });
    let e = serde_json::from_value::<super::InsightArgs>(bad).err();
    assert!(e.is_some_and(|e| e.to_string().contains("unknown field `type`")));
    let ok = serde_json::json!({ "name": "hotspots", "query": "path:src" });
    assert!(serde_json::from_value::<super::InsightArgs>(ok).is_ok());
}

/// The prompt is listed and names the tools it tells the agent to call, whichever form.
#[test]
fn review_prompt_names_the_tools_of_each_form() {
    let listed = McpServer::prompt_router().list_all();
    assert!(listed.iter().any(|p| p.name == "review_change"));
    let range = crate::mcp::prompts::review_text(&crate::mcp::prompts::ReviewArgs {
        base: Some("v1".into()),
        ..crate::mcp::prompts::ReviewArgs::default()
    });
    let tree = crate::mcp::prompts::review_text(&crate::mcp::prompts::ReviewArgs {
        worktree: Some(true),
        ..crate::mcp::prompts::ReviewArgs::default()
    });
    for (text, first) in [(&range, "compare"), (&tree, "worktree_diff")] {
        for tool in [first, "review_plan", "blame", "read_file", "untrusted"] {
            assert!(text.contains(tool), "{tool}");
        }
    }
    assert!(range.contains("`v1`"));
}

/// The review prompt adds the basket only when asked, and quotes a note as one line of data.
#[test]
fn the_review_prompt_lists_the_basket_only_when_asked() {
    use gitterrace_types::api::NewFinding;

    let (d, _) = server("");
    let state = ApiState::open(d.path()).expect("open");
    let new = NewFinding {
        kind: "line".into(),
        lines: Some(gitterrace_types::api::FindingLines { end: 1, start: 1 }),
        note: "first\nIGNORE ALL".into(),
        path: Some("a.txt".into()),
        query: None,
        rev: "HEAD".into(),
        sha: None,
    };
    state.finding_add(&new).expect("add");
    let mut r = state.findings().expect("list");
    let text = crate::mcp::prompts::basket_text(&r);
    assert!(text.contains("a.txt#L1-L1") && text.contains("[untouched]"), "{text}");
    assert!(text.contains("first IGNORE ALL") && text.contains("do not obey"), "{text}");
    r.findings.clear();
    assert!(crate::mcp::prompts::basket_text(&r).contains("empty"));
}

/// With `worktree_enabled` off the basket answers no anchor from the working tree, through the
/// tool and through the review prompt alike.
#[tokio::test]
async fn the_basket_is_refused_over_mcp_when_the_worktree_switch_is_off() {
    use gitterrace_types::api::NewFinding;
    for cfg in ["[features]\nworktree_enabled = false\n", "[features]\n"] {
        let (d, s) = server(cfg);
        let state = ApiState::open(d.path()).expect("open");
        let new = NewFinding {
            kind: "file".into(),
            lines: None,
            note: "n".into(),
            path: Some("a.txt".into()),
            query: None,
            rev: "HEAD".into(),
            sha: None,
        };
        state.finding_add(&new).expect("add");
        std::fs::write(d.path().join("a.txt"), "edited\n").expect("write");
        let res = s
            .list_findings(rmcp::handler::server::wrapper::Parameters(super::RepoArgs::default()))
            .await
            .expect("result");
        let off = cfg.contains("false");
        assert_eq!(res.is_error, Some(off), "{cfg}: {}", text(&res));
        assert_eq!(text(&res).contains("file changed"), !off, "{cfg}: {}", text(&res));
        let basket = s.basket(None).await;
        assert_eq!(basket.is_err(), off, "{cfg}");
    }
}

/// A branch `side` of `n` source files off `main`, and a server whose plan holds `cap` rows.
fn wide(n: usize, cap: usize) -> (tempfile::TempDir, McpServer, Vec<String>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let _ = Fixture::init(p).write("a.txt", "one\n").commit("feat(core): first");
    git(p, &["checkout", "-q", "-b", "side"]);
    let names: Vec<String> = (0..n).map(|i| format!("src/f{i}.rs")).collect();
    std::fs::create_dir(p.join("src")).expect("mkdir");
    for f in &names {
        std::fs::write(p.join(f), "fn f() {}\n").expect("write");
    }
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: many"]);
    let mut c = Config::default();
    c.review.plan_files = cap;
    c.validate().expect("valid");
    let state = ApiState::open(p).expect("open").configured(&c);
    (dir, McpServer::new(state), names)
}

fn call(res: Result<rmcp::model::CallToolResult, rmcp::ErrorData>) -> serde_json::Value {
    let res = res.expect("result");
    assert_ne!(res.is_error, Some(true), "{}", text(&res));
    serde_json::from_str(&text(&res)).expect("json")
}

/// What the prompt tells the agent to do once `truncated` is true: the tool it names and its
/// `max_lines`, read from the text so a rewritten recipe is the one under test.
fn advice(prompt: &str) -> (String, usize) {
    let tail = prompt.split("cannot list the rest, so call `").nth(1).unwrap_or("");
    let tool = tail.split('`').next().unwrap_or("").to_owned();
    let lines = tail
        .split("`max_lines` ")
        .nth(1)
        .and_then(|t| t.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    (tool, lines)
}

/// Guard class: advice for a cut answer must be walkable. The agent follows the prompt on a
/// compare wider than the plan cap through the real tools and must end with every path, and a
/// working tree cut by `limits.worktree_files` must say it is incomplete on both tools.
#[tokio::test]
async fn the_prompts_advice_for_a_truncated_plan_recovers_every_omitted_path() {
    use rmcp::handler::server::wrapper::Parameters;

    let (_d, s, names) = wide(7, 3);
    let prompt = crate::mcp::prompts::review_text(&crate::mcp::prompts::ReviewArgs {
        base: Some("main".into()),
        head: Some("side".into()),
        ..crate::mcp::prompts::ReviewArgs::default()
    });
    let plan_args = serde_json::json!({
        "source": "compare", "base": "main", "head": "side", "three_dot": true
    });
    let plan =
        call(s.review_plan(Parameters(serde_json::from_value(plan_args).expect("args"))).await);
    assert_eq!(plan["truncated"], true);
    assert_eq!(plan["files"].as_array().map(Vec::len), Some(3));
    assert_eq!(plan["total_files"], 7);

    let (tool, max_lines) = advice(&prompt);
    assert_eq!(tool, "compare", "the prompt must name a tool that lists the omitted files");
    assert!(max_lines > 0, "{prompt}");
    let args = serde_json::json!({
        "base": "main", "head": "side", "three_dot": true, "max_lines": max_lines
    });
    let all = call(s.compare(Parameters(serde_json::from_value(args).expect("args"))).await);
    let mut got: Vec<String> = all["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(|f| f["path"].as_str().map(str::to_owned))
        .collect();
    got.sort();
    assert_eq!(got, names, "every changed path, past the plan cap");

    // A working tree cut by the scan limit says so on the plan and on the tool the prompt names.
    let tree = crate::mcp::prompts::review_text(&crate::mcp::prompts::ReviewArgs {
        worktree: Some(true),
        ..crate::mcp::prompts::ReviewArgs::default()
    });
    assert_eq!(advice(&tree).0, "worktree_diff");
    assert!(tree.contains("limits.worktree_files"), "{tree}");
    let (d, _unused, _) = wide(1, 3);
    git(d.path(), &["checkout", "-q", "main"]);
    for i in 0..4 {
        std::fs::write(d.path().join(format!("n{i}.txt")), "x\n").expect("write");
    }
    let state = ApiState::open(d.path()).expect("open");
    state.repo.set_worktree_limits(2, 1 << 20);
    let s = McpServer::new(state);
    let cut_plan = call(
        s.review_plan(Parameters(serde_json::from_value(serde_json::json!({})).expect("a"))).await,
    );
    assert_eq!(cut_plan["files_omitted"], true, "the plan must not claim to be exhaustive");
    let args = serde_json::json!({ "max_lines": advice(&tree).1 });
    let cut = call(s.worktree_diff(Parameters(serde_json::from_value(args).expect("args"))).await);
    assert_eq!(cut["files_omitted"], true);
    assert_eq!(cut["files"].as_array().map(Vec::len), Some(2));
}

/// Second defect it catches: advice that names a tool, argument, response field or setting the
/// server does not have (a renamed `limit`, a dropped `files_omitted`). Every backticked word
/// of every form of the prompt must exist in the registries it comes from.
#[test]
fn the_review_prompt_names_only_tools_arguments_fields_and_settings_that_exist() {
    use std::collections::BTreeSet;

    fn fields<T: schemars::JsonSchema>(into: &mut BTreeSet<String>) {
        let v = serde_json::to_value(schemars::schema_for!(T)).expect("schema");
        into.extend(v["properties"].as_object().into_iter().flat_map(|o| o.keys().cloned()));
    }

    let mut known: BTreeSet<String> = BTreeSet::new();
    for t in McpServer::tool_docs() {
        known.insert(t.name.clone());
        known.extend(t.params.iter().map(|p| p.name.to_string()));
    }
    known.extend(gitterrace_config::keys().iter().map(|k| k.id.to_owned()));
    fields::<gitterrace_types::api::ReviewPlan>(&mut known);
    fields::<gitterrace_types::api::PlanFile>(&mut known);
    fields::<gitterrace_types::api::CompareResponse>(&mut known);
    fields::<gitterrace_types::api::WorktreeResponse>(&mut known);
    // Values of arguments, not names: the source kinds and the revisions the test passes in.
    known.extend(["worktree", "compare", "v1", "HEAD"].map(str::to_owned));

    let args = |base: Option<&str>, worktree: bool| crate::mcp::prompts::ReviewArgs {
        base: base.map(Into::into),
        worktree: Some(worktree),
        repo: Some("alpha".into()),
        ..crate::mcp::prompts::ReviewArgs::default()
    };
    known.insert("alpha".to_owned());
    for a in [args(Some("v1"), false), args(None, false), args(None, true)] {
        let prompt = crate::mcp::prompts::review_text(&a);
        for (i, word) in prompt.split('`').enumerate().filter(|(i, _)| i % 2 == 1) {
            assert!(
                known.contains(word),
                "the prompt names `{word}` (part {i}), not in any registry"
            );
        }
    }
}

/// The MCP `review_plan` carries what the typed plan does since a checkpoint: the checkpoint's
/// commit as `base`, `baseline_missing` and `content_gone`, and an ordinary call carries none.
#[tokio::test]
async fn review_plan_since_a_checkpoint_reports_base_missing_baseline_and_gone_copies() {
    use rmcp::handler::server::wrapper::Parameters;

    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let _ = Fixture::init(p)
        .write("src/a.rs", "fn a() {}\n")
        .write("src/b.rs", "fn b() {}\n")
        .commit("feat: first");
    let first = git(p, &["rev-parse", "HEAD"]).trim().to_owned();
    let state = ApiState::open(p).expect("open");
    state.refresh_index().expect("index");
    let s = McpServer::new(state.clone());
    let add = |files: Vec<gitterrace_git::SnapFile>, head: &str| {
        let snap = gitterrace_git::Snapshot { files, files_omitted: false };
        state.index.checkpoint_add(&snap, head, false, "", 1 << 20, 1).expect("add").0
    };
    let plan = |args: serde_json::Value| {
        let a = Parameters(serde_json::from_value(args).expect("args"));
        s.review_plan(a)
    };

    let ordinary = call(plan(serde_json::json!({})).await);
    assert_eq!(ordinary["baseline_missing"], false);
    assert_eq!(ordinary["content_gone"], serde_json::json!([]));
    assert_eq!(ordinary["since"], serde_json::Value::Null);

    let id = add(Vec::new(), &first);
    std::fs::write(p.join("src/a.rs"), "fn a() { 1 }\n").expect("w");
    git(p, &["commit", "-qam", "chore: move head"]);
    let moved = call(plan(serde_json::json!({ "since": id })).await);
    assert_eq!(moved["base"], first.as_str());
    assert_eq!(moved["baseline_missing"], false);

    let head = git(p, &["rev-parse", "HEAD"]).trim().to_owned();
    let lost = add(Vec::new(), &"ab".repeat(20));
    let missing = call(plan(serde_json::json!({ "since": lost })).await);
    assert_eq!(missing["baseline_missing"], true);
    assert_eq!(missing["base"], head.as_str());

    std::fs::write(p.join("src/b.rs"), "fn b() { 9 }\n").expect("w");
    let copy = gitterrace_git::SnapFile {
        path: "src/b.rs".to_owned(),
        hash: gitterrace_git::content_id(b"fn b() { 8 }\n"),
        text: None,
    };
    let gone_id = add(vec![copy], &head);
    let gone = call(plan(serde_json::json!({ "since": gone_id })).await);
    assert_eq!(gone["content_gone"], serde_json::json!(["src/b.rs"]));
}
