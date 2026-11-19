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

/// Files whose product part `found` flags, as `path: finding`.
///
/// Scans `root`'s `dirs` for files with one of `exts` (any when empty). Files named in
/// `owners` (the one place the markup may live) and `*tests.rs` are skipped.
#[must_use]
pub fn outside_owners(
    root: &Path,
    dirs: &[&str],
    exts: &[&str],
    owners: &[&str],
    found: impl Fn(&str) -> Vec<String>,
) -> Vec<String> {
    let mut bad = Vec::new();
    for p in files_in(root, dirs, exts) {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if owners.contains(&name.as_str()) || name.ends_with("tests.rs") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        for f in found(product(&text)) {
            bad.push(format!("{}: {f}", p.display()));
        }
    }
    bad
}

/// The product part of `text`: everything before its first `#[cfg(test)]`, so a test module
/// that spells markup back is not counted as product code.
#[must_use]
pub fn product(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn markup_outside_its_owner_is_reported_and_the_owner_is_not() {
        let dir = tempfile::tempdir().expect("tmp");
        let t = dir.path().join("t");
        std::fs::create_dir_all(&t).expect("dir");
        std::fs::write(t.join("own.html"), "X").expect("w");
        std::fs::write(t.join("page.html"), "X").expect("w");
        std::fs::write(t.join("a.rs"), "fn a() {}\n#[cfg(test)]\nX").expect("w");
        let found = |c: &str| {
            c.contains('X')
                .then(|| "X".to_owned())
                .into_iter()
                .collect()
        };
        let bad = super::outside_owners(dir.path(), &["t"], &[], &["own.html"], found);
        assert_eq!(bad, [format!("{}: X", t.join("page.html").display())]);
    }
}
