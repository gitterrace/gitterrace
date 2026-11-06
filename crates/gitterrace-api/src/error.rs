use std::io;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Everything an API call can fail with.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error(transparent)]
    Git(#[from] gitterrace_git::Error),
    #[error("index: {0}")]
    Index(#[from] io::Error),
    #[error("worker task failed: {0}")]
    Task(String),
    /// The revision has more commits outside the index than one view reads from git.
    #[error(
        "insights for this revision need a reindex: it is further from the indexed branch than \
         one view reads from git (cap {cap}, counted {read})"
    )]
    NeedsReindex { read: usize, cap: usize },
    /// The insights cache failed a read while the answer was built: it is rebuilt, never
    /// answered from.
    #[error(transparent)]
    Unreadable(#[from] gitterrace_index::feed::SourceError),
    /// A newer build owns the cache: this one never writes it, so it cannot rebuild it.
    #[error("the insights cache has format {0}, written by a newer gitterrace")]
    NewerCache(String),
}

/// Every `error.code` an answer can carry, with its HTTP status and meaning. The docs and the
/// `OpenAPI` schema are rendered from this list.
pub const ERROR_CODES: &[(&str, u16, &str)] = &[
    ("bad_request", 400, "a parameter is missing or invalid"),
    ("rev_not_found", 404, "the revision does not exist"),
    (
        "path_not_found",
        404,
        "the path does not exist at the revision",
    ),
    ("git", 500, "reading the repository failed"),
    ("index", 500, "reading the insights cache failed"),
    ("internal", 500, "the server failed"),
    (
        "index_unreadable",
        503,
        "the insights cache failed a read and is being rebuilt; retry shortly",
    ),
    (
        "index_newer",
        503,
        "a newer gitterrace owns the insights cache; run it, or delete the cache to rebuild",
    ),
    (
        "needs_reindex",
        409,
        "the revision is too far from the indexed branch to answer without a reindex",
    ),
];

/// `Retry-After` of a 503, in seconds: a small repository rebuilds within it, and a client that
/// comes back too early gets another 503.
pub const RETRY_AFTER_SECS: &str = "2";

/// Body text of every 5xx answer. The inner error names files on the host (cache and pack
/// paths), so it goes to the server log only.
pub const SERVER_ERROR_MESSAGE: &str = "internal error, the server log has the details";

impl ApiError {
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Git(
                gitterrace_git::Error::RevNotFound(_) | gitterrace_git::Error::PathNotFound(_),
            ) => StatusCode::NOT_FOUND,
            Self::NeedsReindex { .. } => StatusCode::CONFLICT,
            Self::Unreadable(_) | Self::NewerCache(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Git(_) | Self::Index(_) | Self::Task(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Git(gitterrace_git::Error::RevNotFound(_)) => "rev_not_found",
            Self::Git(gitterrace_git::Error::PathNotFound(_)) => "path_not_found",
            Self::Git(_) => "git",
            Self::Index(_) => "index",
            Self::Task(_) => "internal",
            Self::NeedsReindex { .. } => "needs_reindex",
            Self::Unreadable(_) => "index_unreadable",
            Self::NewerCache(_) => "index_newer",
        }
    }
}

/// Every error reply: `{"error": {"code", "message"}}`, the code from [`ApiError::code`].
#[derive(serde::Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(serde::Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let message = if status.is_server_error() {
            tracing::error!(code = self.code(), "api: {self}");
            SERVER_ERROR_MESSAGE.to_owned()
        } else {
            self.to_string()
        };
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code(),
                message,
            },
        };
        let mut res = (status, Json(body)).into_response();
        // A 503 is a cache that cannot answer yet: a client retries instead of giving up.
        if status == StatusCode::SERVICE_UNAVAILABLE {
            res.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_static(RETRY_AFTER_SECS),
            );
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiError, ERROR_CODES};

    #[test]
    fn every_error_is_in_the_documented_codes() {
        let all = [
            ApiError::BadRequest(String::new()),
            ApiError::Git(gitterrace_git::Error::RevNotFound(String::new())),
            ApiError::Git(gitterrace_git::Error::PathNotFound(String::new())),
            ApiError::Git(gitterrace_git::Error::NotARepo(String::new())),
            ApiError::Git(gitterrace_git::Error::Git(String::new())),
            ApiError::Index(std::io::Error::other("x")),
            ApiError::Task(String::new()),
            ApiError::NeedsReindex { read: 0, cap: 0 },
            ApiError::Unreadable(gitterrace_index::feed::SourceError(String::new())),
            ApiError::NewerCache(String::new()),
        ];
        for e in &all {
            assert!(
                ERROR_CODES
                    .iter()
                    .any(|(c, s, _)| *c == e.code() && *s == e.status().as_u16()),
                "{} ({}) is not documented",
                e.code(),
                e.status()
            );
        }
    }
}
