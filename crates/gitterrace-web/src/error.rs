//! Request errors and their HTTP mapping.

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Git(#[from] gitterrace_git::Error),
    #[error("index: {0}")]
    Index(#[from] std::io::Error),
    /// A named thing that is not there (a checkpoint the cap dropped).
    #[error("{0}")]
    Missing(String),
    #[error("render: {0}")]
    Render(#[from] askama::Error),
    #[error("task: {0}")]
    Join(#[from] tokio::task::JoinError),
    /// The insights cache failed a read while a page was built from it. The page is never
    /// shown: the cache is rebuilt and the visitor gets the indexing page meanwhile.
    #[error("{0}")]
    Source(#[from] gitterrace_index::feed::SourceError),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Git(
                gitterrace_git::Error::RevNotFound(_) | gitterrace_git::Error::PathNotFound(_),
            )
            | Self::Missing(_) => StatusCode::NOT_FOUND,
            Self::Source(_) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        // Server-side detail (paths, gix internals) stays in the log.
        let shown = if status == StatusCode::SERVICE_UNAVAILABLE {
            tracing::warn!("{self}");
            "the insights cache is being rebuilt, try again in a moment".to_owned()
        } else if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("{self}");
            "internal error".to_owned()
        } else {
            self.to_string()
        };
        if status == StatusCode::NOT_FOUND {
            return not_found(&shown);
        }
        (status, page(status, &shown)).into_response()
    }
}

#[derive(askama::Template)]
#[template(path = "error_page.html")]
struct ErrorPage {
    status: u16,
    head: crate::pagehead::PageHead,
}

/// The page every error answers with: the app's header, favicon and styles, the status, one
/// line of detail and a way back. One shell, so no error page ships bare.
pub(crate) fn page(status: StatusCode, detail: &str) -> Html<String> {
    use askama::Template;
    let head = crate::pagehead::PageHead::new(
        status.as_u16().to_string(),
        crate::pagehead::Tagline::of(detail),
    );
    let shown = ErrorPage { status: status.as_u16(), head }.render();
    Html(shown.unwrap_or_else(|_| format!("{} {detail}", status.as_u16())))
}

/// A 404 with [`page`], for a path or repository nothing serves. It carries its detail, so a
/// repository's router can show it inside the app shell (`lost::in_shell`).
pub(crate) fn not_found(detail: &str) -> Response {
    let mut res = (StatusCode::NOT_FOUND, page(StatusCode::NOT_FOUND, detail)).into_response();
    res.extensions_mut().insert(crate::lost::Missing(detail.to_owned()));
    res
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
