//! Which files of a commit are generated: `linguist-generated` in `.gitattributes` at that
//! commit, else a fixed list of lockfiles. Diff and search pages fold these files so the
//! hand-written change stays on top.

use std::collections::HashSet;

use gix::bstr::ByteSlice;

use crate::error::git;
use crate::{Repo, Result};

/// File names that are generated wherever they sit, unless `.gitattributes` says otherwise.
const LOCKFILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "go.sum",
    "poetry.lock",
    "Gemfile.lock",
    "composer.lock",
];

const ATTRIBUTE: &str = "linguist-generated";

/// Whether `path` is a lockfile by name alone.
#[must_use]
pub fn is_lockfile(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    LOCKFILES.contains(&name)
}

/// What `linguist-generated` says for one path: `Some(true)` set, `Some(false)` unset
/// (`-linguist-generated` or `=false`), `None` unspecified.
fn attribute(state: gix::attrs::StateRef<'_>) -> Option<bool> {
    match state {
        gix::attrs::StateRef::Set => Some(true),
        gix::attrs::StateRef::Unset => Some(false),
        gix::attrs::StateRef::Value(v) => Some(v.as_bstr() != b"false".as_bstr()),
        gix::attrs::StateRef::Unspecified => None,
    }
}

/// The generated ones among `paths`, read with the `.gitattributes` of commit `rev`. An
/// explicit attribute wins over the lockfile list, so `-linguist-generated` unfolds a lockfile.
pub fn generated_paths<'p>(
    repo: &Repo,
    rev: &str,
    paths: impl IntoIterator<Item = &'p str>,
) -> Result<HashSet<String>> {
    let r = repo.repo();
    let id = repo.resolve_id(&r, rev)?;
    let tree = r.find_commit(id).map_err(git)?.tree_id().map_err(git)?.detach();
    let index = r.index_from_tree(&tree).map_err(git)?;
    let mut stack = r
        .attributes_only(&index, gix::worktree::stack::state::attributes::Source::IdMapping)
        .map_err(git)?;
    let mut out = stack.selected_attribute_matches([ATTRIBUTE]);
    let mut found = HashSet::new();
    for path in paths {
        let platform = stack.at_entry(path, None).map_err(git)?;
        platform.matching_attributes(&mut out);
        let said = out.iter_selected().next().and_then(|m| attribute(m.assignment.state));
        if said.unwrap_or_else(|| is_lockfile(path)) {
            found.insert(path.to_owned());
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::{generated_paths, is_lockfile};

    #[test]
    fn lockfiles_match_by_name_in_any_directory() {
        assert!(is_lockfile("Cargo.lock"));
        assert!(is_lockfile("web/package-lock.json"));
        assert!(!is_lockfile("src/Cargo.lock.rs"));
        assert!(!is_lockfile("main.rs"));
    }

    /// Red when the attribute is ignored (gen.rs open) or the override is ignored (the
    /// unfolded lockfile folded), and when the attributes are read from a different commit.
    #[test]
    fn attributes_at_the_commit_decide_over_the_lockfile_list() -> crate::Result<()> {
        let dir = tempfile::tempdir().map_err(crate::error::git)?;
        let f = gitterrace_testkit::Fixture::init(dir.path())
            .write(
                ".gitattributes",
                "gen.rs linguist-generated\nsub/yarn.lock -linguist-generated\n",
            )
            .write("gen.rs", "x\n")
            .write("main.rs", "x\n")
            .write("Cargo.lock", "x\n")
            .write("sub/yarn.lock", "x\n")
            .commit("one");
        let first = f.git(&["rev-parse", "HEAD"]).trim().to_owned();
        let _f = f.write(".gitattributes", "main.rs linguist-generated=true\n").commit("two");
        let repo = crate::Repo::discover(dir.path())?;
        let all = ["gen.rs", "main.rs", "Cargo.lock", "sub/yarn.lock"];
        let mut at_first: Vec<String> = generated_paths(&repo, &first, all)?.into_iter().collect();
        at_first.sort();
        assert_eq!(at_first, ["Cargo.lock", "gen.rs"]);
        let mut at_head: Vec<String> = generated_paths(&repo, "HEAD", all)?.into_iter().collect();
        at_head.sort();
        assert_eq!(at_head, ["Cargo.lock", "main.rs", "sub/yarn.lock"]);
        Ok(())
    }
}
