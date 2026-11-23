//! Read-only JSON API (axum router) and MCP server over gitterrace-git and gitterrace-index.
//!
//! Both surfaces call the same [`ApiState`] methods, so they return the same data.

#![expect(clippy::unused_async_trait_impl, reason = "the trait fixes the async signature")]

mod bounds;
mod core;
mod endpoints;
mod error;
mod http;
mod insights;
mod mcp;
mod wire;

pub use core::{ApiState, Pages, PlanRequest, Rebuild, options_from};
pub use endpoints::{ENDPOINTS, Endpoint, generator as schema_generator};
pub use error::{ApiError, ERROR_CODES, RETRY_AFTER_SECS, SERVER_ERROR_MESSAGE};
pub use gitterrace_types::api::ParamDoc as Param;
pub use http::{index_body, router};
pub use insights::{
    INSIGHTS, Insight, OWNERSHIP_DEPTH, Params, index as insight_index, list as insight_list,
};
pub use mcp::{McpRepo, McpServer, ToolDoc, serve_mcp, serve_mcp_repos, tool_on};

#[cfg(test)]
mod compare_tests;
#[cfg(test)]
mod limit_tests;
#[cfg(test)]
mod memo_tests;
#[cfg(test)]
mod tests;
