//! The `/raw` route: blob bytes with an `ETag`, refused above `limits.raw_mb`.

use axum::{
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::routes::Shared;
use crate::{app::blocking, error::Result};

/// Whether a blob of `size` bytes is too big for `/raw` under `limits.raw_mb`.
pub(crate) const fn raw_refused(size: u64, raw_mb: u64) -> bool {
    size > raw_mb << 20
}

/// Whether an `If-None-Match` header lists `tag` (or `*`).
fn tag_matches(headers: &axum::http::HeaderMap, tag: &str) -> bool {
    headers.get(header::IF_NONE_MATCH).is_some_and(|v| {
        v.as_bytes()
            .split(|b| *b == b',')
            .any(|t| t.trim_ascii() == tag.as_bytes() || t.trim_ascii() == b"*")
    })
}

/// What `/raw` found: the blob is too large, the client's copy is current, or the bytes.
enum Raw {
    TooLarge,
    Fresh(String),
    Bytes(Vec<u8>, String),
}

/// Serves blob bytes for relative images. Content is untrusted, so scripts are disabled.
/// The `ETag` is the blob id, so a repeat visit costs a 304 and no content read.
pub(super) async fn raw(
    State(app): Shared,
    headers: axum::http::HeaderMap,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Response> {
    let name = path.trim_matches('/').to_owned();
    let limit = app.cfg().limits.raw_mb;
    let loaded = blocking(&app, move |a| {
        // One resolve and one tree walk; the header size comes before any content read.
        let blob = a.repo.raw_blob(&rev, &name, |id, size| {
            raw_refused(size, limit) || tag_matches(&headers, &format!("\"{id}\""))
        })?;
        let tag = format!("\"{}\"", blob.id);
        Ok(match blob.bytes {
            Some(bytes) => Raw::Bytes(bytes, tag),
            None if raw_refused(blob.size, limit) => Raw::TooLarge,
            None => Raw::Fresh(tag),
        })
    })
    .await?;
    let (bytes, tag) = match loaded {
        Raw::TooLarge => {
            // 413 names the request body, and 5xx reads as a server fault worth retrying: the
            // file exists but this server does not hand it out, so 404 with the reason.
            let why = format!(
                "This file is larger than limits.raw_mb ({limit} MB) and is not served raw."
            );
            return Ok(crate::error::not_found(&why));
        }
        Raw::Fresh(tag) => {
            let tag =
                HeaderValue::from_str(&tag).unwrap_or_else(|_| HeaderValue::from_static("\"0\""));
            return Ok((
                StatusCode::NOT_MODIFIED,
                [
                    (header::ETAG, tag),
                    (
                        header::CACHE_CONTROL,
                        HeaderValue::from_static("private, max-age=300"),
                    ),
                ],
            )
                .into_response());
        }
        Raw::Bytes(bytes, tag) => (bytes, tag),
    };
    Ok(media_response(&path, bytes, Some(&tag)))
}

/// Bytes of a file with the content type its name says, the sandbox for anything that could
/// run (SVG, text) and `nosniff`: one answer for `/raw` and `/worktree/raw`.
fn media_response(path: &str, bytes: Vec<u8>, tag: Option<&str>) -> Response {
    // PDFs never open in the browser's viewer on this origin.
    let disposition = if crate::media::kind(path) == Some(crate::media::Kind::Pdf) {
        "attachment"
    } else {
        "inline"
    };
    let (kind, plain) = (
        crate::media::content_type(path),
        crate::media::plain_media(path),
    );
    // The sandbox keeps SVG and text scriptless. It also stops the browser's PDF viewer, so
    // formats that run no author code are served without it.
    let csp = if plain {
        "default-src 'none'; style-src 'unsafe-inline'"
    } else {
        "sandbox; default-src 'none'; style-src 'unsafe-inline'"
    };
    let mut res = (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(kind)),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(csp),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static(disposition),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        ],
        bytes,
    )
        .into_response();
    let h = res.headers_mut();
    match tag {
        Some(tag) => {
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=300"),
            );
            h.insert(
                header::ETAG,
                HeaderValue::from_str(tag).unwrap_or_else(|_| HeaderValue::from_static("\"0\"")),
            );
        }
        // The working copy changes under the same URL: never kept.
        None => {
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        }
    }
    res
}

/// `/worktree/raw/{path}`: the bytes on disk of one changed file, for the working tree page's
/// image previews. Only a changed file is read, so an ignored file, `.git` and `..` paths read
/// as not found; a public server never routes here (`public.rs`).
pub(crate) async fn worktree_raw(State(app): Shared, Path(path): Path<String>) -> Result<Response> {
    let name = path.trim_matches('/').to_owned();
    let limit = app.cfg().limits.raw_mb << 20;
    let bytes = blocking(&app, move |a| {
        Ok(gitterrace_git::worktree_bytes(&a.repo, &name, limit)?)
    })
    .await?;
    Ok(bytes.map_or_else(
        || crate::error::not_found("This file is not a changed file of the working tree."),
        |b| media_response(&path, b, None),
    ))
}
