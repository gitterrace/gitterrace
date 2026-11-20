//! Which repositories `git terrace mcp` may read, and under which names.
//!
//! The command line's repositories always, and with `served` also every repository in the
//! registry (the ones this user served before, which a running server has recorded). Names are
//! the web server's `/r/NAME` when the registry holds one, so an agent and a browser agree.
//! Nothing here writes the registry: an MCP session claims no names.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gitterrace_api::McpRepo;
use gitterrace_git::Repo;

use crate::multi::{url_stem, valid_name};
use crate::registry::{Entry, candidates, pick, state_dir};

/// The MCP scope: a line saying where it comes from, and the repositories in it.
pub fn mcp_scope(paths: &[PathBuf], served: bool) -> (String, Vec<McpRepo>) {
    let recorded = if served || paths.len() > 1 {
        state_dir().and_then(|d| candidates(&d).ok()).unwrap_or_default()
    } else {
        Vec::new()
    };
    scope_from(paths, served, &recorded)
}

/// [`mcp_scope`] over an already read registry.
pub(crate) fn scope_from(
    paths: &[PathBuf],
    served: bool,
    recorded: &[Entry],
) -> (String, Vec<McpRepo>) {
    let mut out: Vec<McpRepo> = Vec::new();
    let mut roots: HashSet<PathBuf> = HashSet::new();
    for p in paths {
        let found = Repo::discover(p).map(|r| r.root());
        let root = found.as_ref().map_or_else(|_| p.clone(), Clone::clone);
        if !roots.insert(root.clone()) {
            continue;
        }
        let unavailable = found.err().map(|e| e.to_string());
        let name = name_for(&root, recorded, &out);
        out.push(McpRepo { name, path: root, unavailable });
    }
    if served {
        for e in recorded {
            if e.id.is_empty() || roots.contains(&e.path) {
                continue;
            }
            roots.insert(e.path.clone());
            let unavailable = check(&e.path);
            let name = name_for(&e.path, recorded, &out);
            out.push(McpRepo { name, path: e.path.clone(), unavailable });
        }
    }
    let scope = if served {
        "the command line and the repositories served before"
    } else {
        "the command line"
    };
    (scope.to_owned(), out)
}

/// Why a recorded folder cannot be read now: gone, or not the top of a repository.
fn check(path: &Path) -> Option<String> {
    if !path.exists() {
        return Some("the folder is gone".to_owned());
    }
    match Repo::discover(path) {
        Ok(r) if r.root() == path => None,
        Ok(_) => Some("the folder is inside another repository".to_owned()),
        Err(e) => Some(format!("not a git repository: {e}")),
    }
}

/// The registry's name for `root` when it is usable and free here, else the folder stem with
/// the same collision rule the web server uses.
fn name_for(root: &Path, recorded: &[Entry], taken: &[McpRepo]) -> String {
    let used = |n: &str| taken.iter().any(|r| r.name.eq_ignore_ascii_case(n));
    if let Some(e) = recorded.iter().find(|e| e.path == root)
        && valid_name(&e.name)
        && !used(&e.name)
    {
        return e.name.clone();
    }
    let folder = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let other = |n: &str| {
        used(n) || recorded.iter().any(|e| e.path != root && e.name.eq_ignore_ascii_case(n))
    };
    pick(&url_stem(&folder), root, &other)
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;

    use super::scope_from;
    use crate::registry::Entry;

    fn repo(root: &std::path::Path, name: &str) -> std::path::PathBuf {
        let p = root.join(name);
        std::fs::create_dir(&p).expect("mkdir");
        git(&p, &["init", "-q", "-b", "main"]);
        git(&p, &["commit", "-q", "--allow-empty", "-m", "first"]);
        p.canonicalize().expect("canonical")
    }

    fn recorded(path: &std::path::Path, name: &str) -> Entry {
        Entry {
            id: "id".to_owned(),
            path: path.to_path_buf(),
            last_served: 0,
            name: name.to_owned(),
        }
    }

    /// Without `served` a recorded repository is outside the scope: never listed, so no tool
    /// can name it. With it, it joins under its recorded web name.
    #[test]
    fn a_served_repository_joins_the_scope_only_when_asked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mine = repo(dir.path(), "mine");
        let other = repo(dir.path(), "other");
        let reg = [recorded(&other, "other-web")];
        let (_, only) = scope_from(std::slice::from_ref(&mine), false, &reg);
        let names: Vec<&str> = only.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["mine"]);
        assert!(only.iter().all(|r| r.path != other));

        let (scope, all) = scope_from(std::slice::from_ref(&mine), true, &reg);
        let names: Vec<&str> = all.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["mine", "other-web"]);
        assert!(scope.contains("served before"), "{scope}");
    }

    /// A recorded folder that is gone, or that is now a plain folder, is listed unavailable
    /// with a reason, and the scope still builds.
    #[test]
    fn a_recorded_folder_that_is_gone_or_not_a_repository_is_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mine = repo(dir.path(), "mine");
        let plain = dir.path().join("plain");
        std::fs::create_dir(&plain).expect("mkdir");
        let plain = plain.canonicalize().expect("canonical");
        let gone = dir.path().join("gone");
        let reg = [recorded(&gone, "gone"), recorded(&plain, "plain")];
        let (_, all) = scope_from(&[mine], true, &reg);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].unavailable.as_deref(), Some("the folder is gone"));
        assert!(all[2].unavailable.is_some(), "{:?}", all[2]);
    }

    /// A name the registry holds that is not URL-safe is not trusted: the folder stem is used.
    #[test]
    fn a_recorded_name_that_is_not_url_safe_is_replaced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let other = repo(dir.path(), "other");
        let (_, all) = scope_from(&[], true, &[recorded(&other, "../x")]);
        assert_eq!(all[0].name, "other");
    }
}
