//! Request errors and their HTTP mapping.

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use crate::fmt::esc;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Git(#[from] gitterrace_git::Error),
    #[error("index: {0}")]
    Index(#[from] std::io::Error),
    #[error("render: {0}")]
    Render(#[from] askama::Error),
    #[error("task: {0}")]
    Join(#[from] tokio::task::JoinError),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Git(
                gitterrace_git::Error::RevNotFound(_) | gitterrace_git::Error::PathNotFound(_),
            ) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("{self}");
        }
        let body = format!(
            "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
             <link rel=stylesheet href=/static/app.css><main class=wrap><h1>{}</h1><p class=muted>{}</p>\
             <p><a href=/>Back to overview</a></p></main>",
            status.as_u16(),
            esc(&self.to_string())
        );
        (status, Html(body)).into_response()
    }
}
