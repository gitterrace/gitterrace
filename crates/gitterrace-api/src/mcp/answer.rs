//! How a tool answers: its value as JSON, or a failure the agent reads and reacts to.

use rmcp::ErrorData;
use rmcp::model::{CallToolResult, ContentBlock};

use crate::error::{ApiError, SERVER_ERROR_MESSAGE};

/// A tool failure the agent reads and reacts to.
pub(super) fn refuse(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text)])
}

/// A tool's outcome as its answer: the value as JSON, or the failure as a result to read.
pub(super) fn answer(
    out: Result<impl serde::Serialize, String>,
) -> Result<CallToolResult, ErrorData> {
    match out {
        Ok(v) => ok_json(&v),
        Err(why) => Ok(refuse(why)),
    }
}

/// A tool answer: the value as JSON text.
pub(super) fn ok_json(v: &impl serde::Serialize) -> Result<CallToolResult, ErrorData> {
    let text =
        serde_json::to_string(v).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// A blocking task's result, its panic or cancellation as a protocol error.
pub(super) fn joined<T>(r: Result<T, tokio::task::JoinError>) -> Result<T, ErrorData> {
    r.map_err(|e| ErrorData::internal_error(e.to_string(), None))
}

/// What an agent reads for a failed tool: the error, except for a server error, whose text
/// can name files on the host and so goes to the log alone, as for the HTTP answer.
pub(super) fn tool_error_text(e: &ApiError) -> String {
    if e.status().is_server_error() {
        tracing::error!(code = e.code(), "mcp: {e}");
        SERVER_ERROR_MESSAGE.to_owned()
    } else {
        e.to_string()
    }
}
