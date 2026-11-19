//! Which repositories the CLI paths name: a repository as is, a folder searched for them.

use std::io::Read;
use std::path::{Path, PathBuf};

/// A working tree (holding `.git`) or a bare repository (`HEAD`, `objects` and `refs` at its
/// top). A bare clone has no `.git`, and searching it for repositories finds none.
///
/// A `.git` folder counts only once it holds `HEAD`, and a `.git` file only when it names a
/// `gitdir:`: a clone still being written has an empty `.git` and must not be served yet.
pub(super) fn is_repo(dir: &Path) -> bool {
    let git = dir.join(".git");
    if git.is_dir() {
        return git.join("HEAD").is_file();
    }
    if git.is_file() {
        return names_gitdir(&git);
    }
    dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir()
}

/// Whether the `.git` file at `path` starts with `gitdir: `, reading at most its first line.
fn names_gitdir(path: &Path) -> bool {
    let mut head = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|()| &head == b"gitdir: ")
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

/// The CLI paths [`discover_repos`] searches as folders.
///
/// Those that are no repository and hold at least one. A server keeps watching them, so a
/// repository deleted or added there leaves or joins without a restart.
pub fn search_roots(paths: &[PathBuf], depth: usize, skip: &[String]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| {
            let mut found = Vec::new();
            !is_repo(p) && {
                walk(p, depth, skip, &mut found);
                !found.is_empty()
            }
        })
        .cloned()
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_written_clone_is_not_a_repository() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let clone = dir.path().join("clone");
        std::fs::create_dir_all(clone.join(".git"))?;
        assert!(!is_repo(&clone), "an empty .git is a clone in progress");
        std::fs::write(clone.join(".git/HEAD"), "ref: refs/heads/main\n")?;
        assert!(is_repo(&clone), "a .git with HEAD is served");

        let linked = dir.path().join("linked");
        std::fs::create_dir(&linked)?;
        std::fs::write(linked.join(".git"), "")?;
        assert!(!is_repo(&linked), "an empty .git file names no gitdir");
        std::fs::write(linked.join(".git"), "gitdir: ../clone/.git\n")?;
        assert!(is_repo(&linked), "a gitdir file is a linked worktree");
        Ok(())
    }
}
