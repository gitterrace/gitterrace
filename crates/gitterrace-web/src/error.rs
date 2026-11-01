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
        // Server-side detail (paths, gix internals) stays in the log.
        let shown = if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("{self}");
            "internal error".to_owned()
        } else {
            self.to_string()
        };
        (status, page(status, &shown)).into_response()
    }
}

/// The page every error answers with: the app's favicon and styles, the status, one line of
/// detail and a way back. One shell, so no error page ships without the icon.
pub(crate) fn page(status: StatusCode, detail: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">{}\
         <link rel=stylesheet href=/static/tokens.css><link rel=stylesheet href=/static/app.css><main class=wrap><h1>{}</h1><p class=muted>{}</p>\
         <p><a href=/>Back to overview</a></p></main>",
        crate::fmt::FAVICON,
        status.as_u16(),
        esc(detail)
    ))
}

/// A 404 with [`page`], for a path or repository nothing serves.
pub(crate) fn not_found(detail: &str) -> Response {
    (StatusCode::NOT_FOUND, page(StatusCode::NOT_FOUND, detail)).into_response()
}

/// Answers every path no route matches: the error page, never an empty body.
pub(crate) async fn fallback() -> Response {
    not_found("Nothing here.")
}

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    use crate::error::Error;

    #[tokio::test]
    async fn server_errors_hide_their_detail_and_not_found_keeps_it() {
        let io = Error::Index(std::io::Error::other("/secret/.git/objects/pack/x"));
        let res = io.into_response();
        assert_eq!(res.status().as_u16(), 500);
        let body = res.into_body().collect().await.expect("body").to_bytes();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("internal error") && !body.contains("/secret"));
        let nf = Error::Git(gitterrace_git::Error::RevNotFound("abc".into())).into_response();
        assert_eq!(nf.status().as_u16(), 404);
        let body = nf.into_body().collect().await.expect("body").to_bytes();
        assert!(String::from_utf8_lossy(&body).contains("abc"));
    }
}
