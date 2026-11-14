//! Reading pages from the router and writing them under the output folder without escaping it.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use gitterrace_git::percent_encode_path;
use tower::ServiceExt;
use unicode_normalization::UnicodeNormalization;

/// Largest page read, in bytes. Not configurable: it bounds one page in memory; a page over it
/// fails the export naming the settings that make pages larger.
const MAX_BODY: usize = 64 * 1024 * 1024;
/// The settings that make a page larger, named when one does not fit.
const SHRINK: &str = "Lower limits.blob_lines, limits.highlight_kb, limits.text_kb or \
                      limits.diff_fold_lines in config.toml";

/// File the page lands in: HTML pages become `<path>/index.html` so their links
/// resolve when the folder is served by any static server. `None` for a path with a segment
/// that is not a plain name (`.`, `..`, a separator or drive colon on some platform, NUL), a
/// Windows device name such as `NUL` or `com1.txt`, or a name ending in a dot or a space,
/// which Windows strips: such a page is skipped rather than written somewhere it was not
/// named. Refused on every platform, since an export is copied and served elsewhere.
pub(super) fn target(out: &Path, path: &str, content_type: &str) -> Option<PathBuf> {
    let mut p = out.to_path_buf();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        let hostile = seg == "." || seg == ".." || seg.contains(['\\', ':', '\0']);
        if hostile || seg.ends_with(['.', ' ']) || is_device(seg) {
            return None;
        }
        p.push(seg);
    }
    if content_type.starts_with("text/html") {
        p.push("index.html");
    }
    Some(p)
}

/// Whether Win32 opens a device for `seg` in any folder: the part before the first dot,
/// trailing spaces dropped, is a reserved name in any case.
fn is_device(seg: &str) -> bool {
    let stem = seg.split('.').next().unwrap_or(seg).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(upper.as_bytes(), [b'C', b'O', b'M', b'1'..=b'9'])
        || matches!(upper.as_bytes(), [b'L', b'P', b'T', b'1'..=b'9'])
}

/// The name a case-insensitive, normalizing file system (APFS, NTFS) stores `name` under:
/// two names with one key land in one file.
pub(super) fn fold(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// Writes one page; `false` when its path is refused by `target` and the page is skipped.
pub(super) fn write_page(
    out: &Path,
    path: &str,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<bool> {
    let Some(file) = target(out, path, content_type) else {
        eprintln!("skipping {path:?}: not a plain file name");
        return Ok(false);
    };
    let root = out
        .canonicalize()
        .with_context(|| format!("resolving {}", out.display()))?;
    // `target` drops `..`, but a symlink already inside `out` could still lead
    // elsewhere. Walk down one component at a time and refuse before creating
    // anything: checking after `create_dir_all` would already have written
    // directories through the link.
    let mut dir = out.to_path_buf();
    if let Some(parent) = file.parent() {
        for part in parent.strip_prefix(out).unwrap_or(parent).components() {
            dir.push(part);
            if !dir.exists() {
                std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
            }
            let real = dir
                .canonicalize()
                .with_context(|| format!("resolving {}", dir.display()))?;
            anyhow::ensure!(
                real.starts_with(&root),
                "refusing to write {} outside {}",
                file.display(),
                out.display()
            );
        }
    }
    // The parents are real directories inside `out`; the leaf itself may still be a link.
    write_leaf(&file, body)?;
    Ok(true)
}

/// Through the shared no-follow temp-and-rename writer: a link at the leaf is refused, never
/// written through, and a crash leaves the old page whole.
pub(super) fn write_leaf(file: &Path, body: &[u8]) -> anyhow::Result<()> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name().and_then(|n| n.to_str())) else {
        bail!("refusing to write {}: no file name", file.display());
    };
    gitterrace_git::write_atomic(dir, name, body, 0o644)
        .with_context(|| format!("refusing to write {}", file.display()))
}

pub(crate) async fn fetch(
    router: &Router,
    path: &str,
) -> anyhow::Result<(StatusCode, String, Vec<u8>)> {
    fetch_within(router, path, MAX_BODY).await
}

/// [`fetch`] with a body limit of `most` bytes, so a test can reach it with a small page.
pub(crate) async fn fetch_within(
    router: &Router,
    path: &str,
    most: usize,
) -> anyhow::Result<(StatusCode, String, Vec<u8>)> {
    // The router refuses requests without a local Host (DNS-rebinding guard).
    // Paths travel decoded (they name files on disk); encode them for the URI.
    let req = Request::get(percent_encode_path(path))
        .header(axum::http::header::HOST, "localhost")
        .body(Body::empty())
        .context("building a request")?;
    let res = router
        .clone()
        .oneshot(req)
        .await
        .context("routing a request")?;
    let status = res.status();
    let content_type = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let body = to_bytes(res.into_body(), most).await.map_err(|_| {
        anyhow::anyhow!(
            "{path} is larger than {} MiB, the most one exported page may be. {SHRINK}",
            most >> 20
        )
    })?;
    Ok((status, content_type, body.to_vec()))
}

/// The files every export holds besides its pages: the snapshot style, the host's headers and
/// the not-found page.
pub(super) async fn write_fixed(
    out: &Path,
    router: &Router,
    snapshot_css: &str,
    headers: &str,
) -> anyhow::Result<()> {
    write_page(out, "/snapshot.css", "text/css", snapshot_css.as_bytes())?;
    write_page(out, "/_headers", "text/plain", headers.as_bytes())?;
    write_not_found(out, router).await
}

/// `404.html` at the root: the server's own page for a path nothing serves, which a static host
/// answers a missing file with. Without it a wrong link got the host's bare error. Written as
/// the root page is, so its styles resolve.
async fn write_not_found(out: &Path, router: &Router) -> anyhow::Result<()> {
    let (status, content_type, body) = fetch(router, "/_gitterrace/not-found").await?;
    anyhow::ensure!(
        status == StatusCode::NOT_FOUND && content_type.starts_with("text/html"),
        "the not-found page answered {status} {content_type}"
    );
    let raw = String::from_utf8_lossy(&body);
    let page = crate::export::fonts::drop_font_preloads(&raw);
    let text = crate::export::relative::relativize(&page, "/", true);
    gitterrace_git::write_atomic(out, "404.html", text.as_bytes(), 0o644)
        .context("writing 404.html")
}
