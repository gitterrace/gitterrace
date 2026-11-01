//! The one way generated output reaches the tree.
//!
//! A generator returns [`Out`] values and never touches the file system. [`apply`] writes them
//! (`generate`) or compares them with the tree (`check`), so both commands run the same code and
//! a check cannot mutate the tree.

use std::path::Path;

use anyhow::{Context as _, Result};

/// One generated file. The path is relative to the repo root.
pub struct Out {
    pub path: String,
    pub body: String,
}

impl Out {
    pub fn file(path: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            body: body.into(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Write,
    Check,
}

/// Write `outs` or compare them with the tree. Returns the paths that differed; in
/// [`Mode::Write`] those are the ones rewritten.
pub fn apply(root: &Path, outs: Vec<Out>, mode: Mode) -> Result<Vec<String>> {
    let mut stale = Vec::new();
    for Out { path, body } in outs {
        let file = root.join(&path);
        let current = match std::fs::read_to_string(&file) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("reading {path}")),
        };
        if current.as_deref() == Some(body.as_str()) {
            continue;
        }
        if mode == Mode::Write {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).with_context(|| format!("creating {path}"))?;
            }
            std::fs::write(&file, &body).with_context(|| format!("writing {path}"))?;
        }
        stale.push(path);
    }
    Ok(stale)
}

#[cfg(test)]
mod tests {
    use crate::out::{Mode, Out, apply};

    #[test]
    fn check_reports_without_writing_and_write_converges() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let outs = || vec![Out::file("docs/x.md", "new\n")];

        let stale = apply(root, outs(), Mode::Check).expect("check");
        assert_eq!(stale, ["docs/x.md"]);
        assert!(!root.join("docs/x.md").exists(), "check wrote to the tree");

        apply(root, outs(), Mode::Write).expect("write");
        assert!(apply(root, outs(), Mode::Check).expect("check").is_empty());
    }
}
