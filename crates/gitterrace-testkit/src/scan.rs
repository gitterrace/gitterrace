//! The source walk the scanner tests share: every file under some folders with one of some
//! extensions, and the product part of a Rust file (before its first `#[cfg(test)]`).

use std::fs;
use std::path::{Path, PathBuf};

/// Folders a walk never enters: build output, packages and the git store.
const SKIP: [&str; 3] = ["target", "node_modules", ".git"];

/// Every file under `dirs` (recursively) whose extension is one of `exts`, or every file when
/// `exts` is empty, sorted. A folder that does not exist adds nothing.
#[must_use]
pub fn files(dirs: &[PathBuf], exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in dirs {
        walk(d, exts, &mut out);
    }
    out.sort();
    out
}

/// [`files`] under `root` joined with each of `dirs`.
#[must_use]
pub fn files_in(root: &Path, dirs: &[&str], exts: &[&str]) -> Vec<PathBuf> {
    files(&dirs.iter().map(|d| root.join(d)).collect::<Vec<_>>(), exts)
}

fn walk(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !SKIP.iter().any(|s| e.file_name() == *s) {
                walk(&p, exts, out);
            }
        } else if exts.is_empty()
            || p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| exts.contains(&x))
        {
            out.push(p);
        }
    }
}

/// The product part of `text`: everything before its first `#[cfg(test)]`, so a test module
/// that spells markup back is not counted as product code.
#[must_use]
pub fn product(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or_default()
}
