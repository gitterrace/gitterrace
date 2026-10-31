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

impl ApiError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Git(gitterrace_git::Error::RevNotFound(_) | gitterrace_git::Error::PathNotFound(_)) => {
                StatusCode::NOT_FOUND
            }
            Self::Git(_) | Self::Index(_) | Self::Task(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
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
        let body =
            serde_json::json!({ "error": { "code": self.code(), "message": self.to_string() } });
        (self.status(), Json(body)).into_response()
    }
}
