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
];

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
        }
    }
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
        let body = serde_json::json!({ "error": { "code": self.code(), "message": message } });
        (status, Json(body)).into_response()
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
