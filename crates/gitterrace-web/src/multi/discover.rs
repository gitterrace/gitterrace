//! Which repositories the CLI paths name: a repository as is, a folder searched for them.

use std::path::{Path, PathBuf};

/// A working tree (holding `.git`) or a bare repository (`HEAD`, `objects` and `refs` at its
/// top). A bare clone has no `.git`, and searching it for repositories finds none.
fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
        || (dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir())
}

/// Expands the CLI paths.
///
/// A repository stays as is, a plain folder is searched `depth` levels
/// down for repositories (skipping hidden folders and those in `skip`), and a folder with none
/// is taken as a path inside one.
pub fn discover_repos(paths: &[PathBuf], depth: usize, skip: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if is_repo(p) {
            out.push(p.clone());
            continue;
        }
        let before = out.len();
        walk(p, depth, skip, &mut out);
        if out.len() == before {
            out.push(p.clone());
        }
    }
    out
}

fn walk(dir: &Path, depth: usize, skip: &[String], out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            !n.starts_with('.') && !skip.iter().any(|s| s.as_str() == n)
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for d in dirs {
        if is_repo(&d) {
            out.push(d);
        } else {
            walk(&d, depth - 1, skip, out);
        }
    }
}
