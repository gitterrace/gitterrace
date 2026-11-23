//! Serving the MCP server over stdin and stdout.

use std::path::Path;

use rmcp::ServiceExt;

use crate::core::ApiState;
use crate::error::ApiError;
use crate::mcp::bounded::{LineBound, MAX_LINE_BYTES};
use crate::mcp::{McpRepo, McpServer};

/// Stdin and stdout, with a line limit on what is read.
fn stdio() -> (LineBound<tokio::io::Stdin>, tokio::io::Stdout) {
    (LineBound::new(tokio::io::stdin(), MAX_LINE_BYTES), tokio::io::stdout())
}

/// Serves MCP on stdin/stdout for the repository containing `repo_path` until the client leaves.
///
/// The insights cache is brought up to date first; when that fails the server still answers
/// with whatever the cache holds, since browsing reads never need it. `cfg` sets the `link`
/// form and the counting defaults, the same way it does for the web's `/api/v1`.
#[expect(
    clippy::print_stderr,
    reason = "a stdio MCP server owns stdout for the protocol, so its startup warning goes to \
        stderr"
)]
pub async fn serve_mcp(repo_path: &Path, cfg: &gitterrace_config::Config) -> Result<(), ApiError> {
    let path = repo_path.to_owned();
    let owned = cfg.clone();
    let state = tokio::task::spawn_blocking(move || {
        let state = ApiState::open(&path)?.configured(&owned);
        if let Err(e) = state.refresh_index() {
            eprintln!("gitterrace: insights index incomplete, serving anyway: {e}");
        }
        Ok::<_, ApiError>(state)
    })
    .await
    .map_err(|e| ApiError::Task(e.to_string()))??;
    let running = McpServer::new(state)
        .with_config(cfg)
        .serve(stdio())
        .await
        .map_err(|e| ApiError::Task(e.to_string()))?;
    running.waiting().await.map_err(|e| ApiError::Task(e.to_string()))?;
    Ok(())
}

/// Serves MCP on stdin/stdout over every repository in `list` until the client leaves.
///
/// Nothing is opened up front: each repository opens (and its insights cache catches up) on
/// the first tool that names it, so a long list starts at once and a broken entry fails alone.
pub async fn serve_mcp_repos(
    scope: &str,
    list: Vec<McpRepo>,
    cfg: &gitterrace_config::Config,
) -> Result<(), ApiError> {
    let running = McpServer::over(scope, list)
        .with_config(cfg)
        .serve(stdio())
        .await
        .map_err(|e| ApiError::Task(e.to_string()))?;
    running.waiting().await.map_err(|e| ApiError::Task(e.to_string()))?;
    Ok(())
}
