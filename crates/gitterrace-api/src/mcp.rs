use std::path::Path;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{ErrorData, ServiceExt, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::core::ApiState;
use crate::error::ApiError;
use crate::insights::Params;

/// The gitterrace MCP server. Every tool is read-only.
#[derive(Clone)]
pub struct McpServer {
    state: ApiState,
}

/// One MCP tool parameter, from the tool's input schema.
#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub ty: String,
    pub required: bool,
    pub description: String,
}

/// One MCP tool as the API reference shows it.
#[derive(Debug)]
pub struct ToolDoc {
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
    pub read_only: bool,
}

/// The JSON type of a schema property. `Option<T>` fields come as `["integer", "null"]`.
fn schema_type(schema: &Value) -> String {
    match schema.get("type") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or("any")
            .to_owned(),
        _ => "any".to_owned(),
    }
}

impl McpServer {
    pub fn new(state: ApiState) -> Self {
        Self { state }
    }

    /// Names of the registered tools, sorted.
    pub fn tool_names() -> Vec<String> {
        let mut names: Vec<String> = Self::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        names
    }

    /// Every tool as documentation data, sorted by name, read from the live registry.
    pub fn tool_docs() -> Vec<ToolDoc> {
        let mut docs: Vec<ToolDoc> = Self::tool_router()
            .list_all()
            .into_iter()
            .map(|t| {
                let required: Vec<&str> = t
                    .input_schema
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                let mut params: Vec<Param> = t
                    .input_schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|props| {
                        props
                            .iter()
                            .map(|(name, schema)| Param {
                                name: name.clone(),
                                ty: schema_type(schema),
                                required: required.contains(&name.as_str()),
                                description: schema
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                params.sort_by(|a, b| a.name.cmp(&b.name));
                ToolDoc {
                    name: t.name.to_string(),
                    description: t.description.as_deref().unwrap_or_default().to_owned(),
                    params,
                    read_only: t
                        .annotations
                        .as_ref()
                        .and_then(|a| a.read_only_hint)
                        .unwrap_or(false),
                }
            })
            .collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        docs
    }

    async fn run<F>(&self, f: F) -> Result<CallToolResult, ErrorData>
    where
        F: FnOnce(&ApiState) -> Result<Value, ApiError> + Send + 'static,
    {
        let state = self.state.clone();
        let out = tokio::task::spawn_blocking(move || f(&state))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        // Tool failures are results the agent can read and react to, not protocol errors.
        Ok(match out {
            Ok(v) => CallToolResult::success(vec![Content::text(v.to_string())]),
            Err(e) => CallToolResult::error(vec![Content::text(e.to_string())]),
        })
    }
}

#[derive(Deserialize, JsonSchema, Default)]
struct RevArgs {
    /// Branch, tag, sha or HEAD. Defaults to HEAD.
    rev: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct LogArgs {
    /// Branch, tag, sha or HEAD. Defaults to HEAD.
    rev: Option<String>,
    /// Only commits touching this path (file or directory).
    path: Option<String>,
    /// 1-based page. Defaults to 1.
    page: Option<usize>,
    /// Commits per page, default 30, max 500.
    per_page: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct ShowArgs {
    /// Commit id, abbreviated id, branch or tag.
    id: String,
    /// Include hunks. Defaults to true. False lists changed files only.
    diff: Option<bool>,
    /// Cap on diff lines returned across all files. Defaults to 400.
    max_lines: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct ReadArgs {
    rev: Option<String>,
    /// Repository-relative file path.
    path: String,
    /// 1-based first line. Defaults to 1.
    start_line: Option<usize>,
    /// Maximum lines, default and max 5000.
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct TreeArgs {
    rev: Option<String>,
    /// Directory path, empty for the repository root.
    path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct BlameArgs {
    rev: Option<String>,
    /// Repository-relative file path.
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct HotspotArgs {
    rev: Option<String>,
    /// Only count changes after this Unix timestamp. Defaults to all time.
    since: Option<i64>,
    /// Files to return. Defaults to 20.
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct ActivityArgs {
    rev: Option<String>,
    /// `day`, `week` or `month`. Defaults to month.
    granularity: Option<String>,
}

#[tool_router(server_handler)]
impl McpServer {
    /// Orientation for a repository: name, HEAD, current branch and totals (commits, authors,
    /// files touched, lines added and removed, first and last commit time). Call this first,
    /// before drilling into history. Use `log` for the commits themselves.
    #[tool(name = "repo_overview", annotations(read_only_hint = true))]
    async fn repo_overview(&self) -> Result<CallToolResult, ErrorData> {
        self.run(ApiState::repo_info).await
    }

    /// Commit history, newest first, with paging. Pass `path` to see only commits that touched a
    /// file or directory ("who changed this and why"). Returns id, parents, author, committer and
    /// subject per commit plus `has_more`. For one commit's diff use `show_commit`.
    #[tool(name = "log", annotations(read_only_hint = true))]
    async fn log(&self, Parameters(a): Parameters<LogArgs>) -> Result<CallToolResult, ErrorData> {
        self.run(move |s| {
            s.log(
                a.rev.as_deref(),
                a.path.as_deref(),
                a.page.unwrap_or(1),
                a.per_page.unwrap_or(30),
            )
        })
        .await
    }

    /// One commit: full message, changed files with additions and deletions and, by default, the
    /// diff hunks (capped by `max_lines`, `truncated` says when cut). Use after `log` or `blame`
    /// to learn what a change did.
    #[tool(name = "show_commit", annotations(read_only_hint = true))]
    async fn show_commit(
        &self,
        Parameters(a): Parameters<ShowArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |s| {
            s.commit(
                &a.id,
                a.diff.unwrap_or(true),
                Some(a.max_lines.unwrap_or(400)),
            )
        })
        .await
    }

    /// File content at a revision, read from git (not the working tree). Returns text, total line
    /// count and whether it was cut. Binary files return `binary: true` and no text. Use
    /// `start_line` and `limit` for large files. Not for directories: use `list_tree`.
    #[tool(name = "read_file", annotations(read_only_hint = true))]
    async fn read_file(
        &self,
        Parameters(a): Parameters<ReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |s| s.blob(a.rev.as_deref(), &a.path, a.start_line, a.limit))
            .await
    }

    /// Directory listing at a revision: name, kind (dir, file, symlink, submodule) and size per
    /// entry, one level deep. Use to explore the layout before `read_file`.
    #[tool(name = "list_tree", annotations(read_only_hint = true))]
    async fn list_tree(
        &self,
        Parameters(a): Parameters<TreeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |s| s.tree(a.rev.as_deref(), a.path.as_deref()))
            .await
    }

    /// Per-line authorship of a file: the commit, author and time that last changed each line.
    /// Use to find who to ask about, or which commit introduced, a line. Follow up with
    /// `show_commit` on the commit id.
    #[tool(name = "blame", annotations(read_only_hint = true))]
    async fn blame(
        &self,
        Parameters(a): Parameters<BlameArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |s| s.blame(a.rev.as_deref(), &a.path)).await
    }

    /// Most frequently changed files (commits, additions, deletions, last change), from the stats
    /// cache. Use to find risky or central code and where to look first. Optional `since`.
    #[tool(name = "hotspots", annotations(read_only_hint = true))]
    async fn hotspots(
        &self,
        Parameters(a): Parameters<HotspotArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = Params {
            since: a.since,
            limit: a.limit,
            ..Params::default()
        };
        self.run(move |s| s.insight("hotspots", a.rev.as_deref(), &p))
            .await
    }

    /// Contributors with commits, lines, first and last commit and active days, most active
    /// first, plus `bus_factor` (the fewest authors who made half of all commits). Use for
    /// ownership and knowledge-concentration questions.
    #[tool(name = "contributors", annotations(read_only_hint = true))]
    async fn contributors(
        &self,
        Parameters(a): Parameters<RevArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(move |s| s.contributors(a.rev.as_deref())).await
    }

    /// Commits and lines changed per day, week or month. Use for "how active is this project" and
    /// trend questions.
    #[tool(name = "activity", annotations(read_only_hint = true))]
    async fn activity(
        &self,
        Parameters(a): Parameters<ActivityArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = Params {
            granularity: a.granularity,
            ..Params::default()
        };
        self.run(move |s| s.insight("activity", a.rev.as_deref(), &p))
            .await
    }
}

/// Serves MCP on stdin/stdout for the repository containing `repo_path` until the client leaves.
/// The insights cache is brought up to date first.
pub async fn serve_mcp(repo_path: &Path) -> Result<(), ApiError> {
    let path = repo_path.to_owned();
    let state = tokio::task::spawn_blocking(move || {
        let state = ApiState::open(&path)?;
        state.refresh_index()?;
        Ok::<_, ApiError>(state)
    })
    .await
    .map_err(|e| ApiError::Task(e.to_string()))??;
    let running = McpServer::new(state)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| ApiError::Task(e.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|e| ApiError::Task(e.to_string()))?;
    Ok(())
}
