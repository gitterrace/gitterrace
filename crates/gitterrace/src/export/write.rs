//! Writing pages under the output folder without escaping it.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// File the page lands in: HTML pages become `<path>/index.html` so their links
/// resolve when the folder is served by any static server. `None` for a path with a segment
/// that is not a plain name (`.`, `..`, a separator or drive colon on some platform, NUL):
/// such a page is skipped rather than written somewhere it was not named.
pub(super) fn target(out: &Path, path: &str, content_type: &str) -> Option<PathBuf> {
    let mut p = out.to_path_buf();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        let hostile = seg == "." || seg == ".." || seg.contains(['\\', ':', '\0']);
        if hostile {
            return None;
        }
        p.push(seg);
    }
    if content_type.starts_with("text/html") {
        p.push("index.html");
    }
    Some(p)
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
