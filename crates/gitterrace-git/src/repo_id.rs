//! A repository's stable id: derived from its root commit, so it survives clones, renames and
//! moves, unlike the folder name. Permalinks carry it to find the served repo again.

use crate::error::git;
use crate::{Repo, Result};

/// Hex digits of the root commit kept in the id. 12 makes an accidental clash between two
/// served repos negligible while keeping permalinks short.
const ID_LEN: usize = 12;

impl Repo {
    /// The repo id: the first 12 hex of the oldest parentless commit reachable from HEAD.
    /// Several roots (a merged-in history) tie-break on the smaller id, so every clone agrees.
    /// `None` for a repository without commits. Computed once per process, it walks history.
    pub fn id(&self) -> Option<String> {
        self.id_cache
            .get_or_init(|| self.root_id().ok().flatten())
            .clone()
    }

    /// [`Repo::id`] when it is already known, else `None` at once: never walks and never
    /// waits for a walk another thread is running. For a page, which must not stall behind it.
    pub fn id_if_known(&self) -> Option<String> {
        self.id_cache.get().cloned().flatten()
    }

    fn root_id(&self) -> Result<Option<String>> {
        let repo = self.repo();
        let Ok(head) = repo.head_id() else {
            return Ok(None);
        };
        let mut best: Option<(i64, gix::ObjectId)> = None;
        for info in repo.rev_walk([head.detach()]).all().map_err(git)? {
            let info = info.map_err(git)?;
            if !info.parent_ids.is_empty() {
                continue;
            }
            let time = info.object().map_err(git)?.time().map_err(git)?.seconds;
            let key = (time, info.id);
            if best.is_none_or(|b| key < b) {
                best = Some(key);
            }
        }
        Ok(best.map(|(_, id)| id.to_hex_with_len(ID_LEN).to_string()))
    }

    /// Whether `sha` names a commit here and, when given, `path` is a file or directory in it.
    /// Read from objects only, so `..` or an absolute path never matches.
    pub fn has(&self, sha: &str, path: Option<&str>) -> bool {
        let repo = self.repo();
        let Ok(id) = gix::ObjectId::from_hex(sha.as_bytes()) else {
            return false;
        };
        let Ok(commit) = repo.find_commit(id) else {
            return false;
        };
        let Some(path) = path.map(|p| p.trim_matches('/')).filter(|p| !p.is_empty()) else {
            return true;
        };
        if path
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
        {
            return false;
        }
        commit
            .tree()
            .ok()
            .and_then(|t| crate::treepath::lookup(&t, path).ok().flatten())
            .is_some()
    }
}

/// The served repo a permalink points at: its id matches and it holds the full `sha` (and
/// `path`). Two clones of one project share an id; the first that has the commit wins.
pub fn resolve_permalink<'a, R: AsRef<Repo>>(
    repos: &'a [R],
    id: &str,
    sha: &str,
    path: Option<&str>,
) -> Option<&'a R> {
    if sha.len() != 40 {
        return None;
    }
    repos.iter().find(|r| {
        let r = r.as_ref();
        r.id().as_deref() == Some(id) && r.has(sha, path)
    })
}

impl AsRef<Self> for Repo {
    fn as_ref(&self) -> &Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git_at;

    use crate::{Repo, resolve_permalink};

    /// Two histories merged: `old` root (2020) on a side branch, `new` root (2024) on main.
    fn two_roots() -> (tempfile::TempDir, String, String) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = tmp.path();
        let t24 = "2024-01-01T00:00:00+00:00";
        let t20 = "2020-01-01T00:00:00+00:00";
        git_at(d, t24, &["init", "-q", "-b", "main"]);
        git_at(d, t24, &["config", "user.name", "T"]);
        git_at(d, t24, &["config", "user.email", "t@example.com"]);
        std::fs::write(d.join("a.txt"), "a\n").expect("write");
        git_at(d, t24, &["add", "-A"]);
        git_at(d, t24, &["commit", "-q", "-m", "new root"]);
        let new = git_at(d, t24, &["rev-parse", "HEAD"]);
        git_at(d, t20, &["checkout", "-q", "--orphan", "side"]);
        git_at(d, t20, &["rm", "-q", "-rf", "."]);
        std::fs::create_dir(d.join("lib")).expect("mkdir");
        std::fs::write(d.join("lib/b.txt"), "b\n").expect("write");
        git_at(d, t20, &["add", "-A"]);
        git_at(d, t20, &["commit", "-q", "-m", "old root"]);
        let old = git_at(d, t20, &["rev-parse", "HEAD"]);
        git_at(d, t24, &["checkout", "-q", "main"]);
        git_at(
            d,
            t24,
            &[
                "merge",
                "-q",
                "--allow-unrelated-histories",
                "-m",
                "merge",
                "side",
            ],
        );
        (tmp, new, old)
    }

    #[test]
    fn id_is_the_oldest_root_and_survives_a_clone() {
        let (tmp, new, old) = two_roots();
        let repo = Repo::discover(tmp.path()).expect("open");
        assert_eq!(repo.id().as_deref(), Some(&old[..12]));
        assert_ne!(repo.id().as_deref(), Some(&new[..12]));

        let clone = tempfile::tempdir().expect("tempdir");
        let dst = clone.path().join("renamed");
        git_at(
            clone.path(),
            "2024-01-01T00:00:00+00:00",
            &[
                "clone",
                "-q",
                &tmp.path().to_string_lossy(),
                &dst.to_string_lossy(),
            ],
        );
        assert_eq!(Repo::discover(&dst).expect("open").id(), repo.id());
    }

    #[test]
    fn empty_repo_has_no_id() {
        let tmp = tempfile::tempdir().expect("tempdir");
        git_at(tmp.path(), "2024-01-01T00:00:00+00:00", &["init", "-q"]);
        assert_eq!(Repo::discover(tmp.path()).expect("open").id(), None);
    }

    #[test]
    fn resolver_matches_id_sha_and_path() {
        let (tmp, _, old) = two_roots();
        let other = tempfile::tempdir().expect("tempdir");
        git_at(other.path(), "2024-01-01T00:00:00+00:00", &["init", "-q"]);
        let repos = vec![
            Repo::discover(other.path()).expect("open"),
            Repo::discover(tmp.path()).expect("open"),
        ];
        let id = repos[1].id().expect("id");
        let head = git_at(
            tmp.path(),
            "2024-01-01T00:00:00+00:00",
            &["rev-parse", "HEAD"],
        );
        let hit = |sha: &str, path| resolve_permalink(&repos, &id, sha, path).is_some();
        assert!(hit(&head, None));
        assert!(hit(&head, Some("lib/b.txt")));
        assert!(hit(&head, Some("lib")));
        assert!(hit(&old, Some("lib/b.txt")));
        assert!(!hit(&old, Some("a.txt")), "a.txt is not in the old root");
        assert!(!hit(&head, Some("../../etc/passwd")));
        assert!(!hit(&head, Some("lib/../a.txt")));
        assert!(!hit(&head[..12], None), "short sha is not a permalink");
        assert!(!hit(&"0".repeat(40), None));
        assert!(resolve_permalink(&repos, "000000000000", &head, None).is_none());
    }
}
