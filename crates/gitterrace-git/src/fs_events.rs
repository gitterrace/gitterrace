//! What file events mean for a repository: which folders to watch and which paths can never
//! change what the working tree scan reports. The watching itself is in `nudge.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::nudge::{Nudge, NudgeConfig, NudgeError, Nudger};
use crate::{IgnoreSet, Repo};

/// Quiet time after the last file event before the scan runs. Long enough to fold an editor's
/// write, rename and chmod into one scan, short enough that a save shows at once.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Longest a build that keeps writing holds the scan back. Not configurable: it only trades a
/// few scans against how soon a long build shows, and `watch.rescan_secs` bounds the rest.
const MAX_DELAY: Duration = Duration::from_millis(500);

/// Folders under the git directory that never change HEAD, a ref, the index or the status:
/// objects arrive before the ref that names them, and gitterrace's own cache lives here.
const GIT_SKIP: [&str; 6] = ["objects", "logs", "gitterrace", "lfs", "hooks", "info"];
/// Worktree folders skipped even when no ignore rule names them: build output and vendored
/// packages that a build rewrites by the thousand.
const WORK_SKIP: [&str; 2] = ["target", "node_modules"];

impl Repo {
    /// Starts OS file events for this repository: the worktree, the git directory and, for a
    /// linked worktree, the shared one; a bare repository has only the git directory. `on_nudge`
    /// only says "look again".
    pub fn file_events(
        &self,
        rescan: Duration,
        os_events: bool,
        on_nudge: impl Fn(Nudge) + Send + 'static,
    ) -> Result<Nudger, NudgeError> {
        let inner = self.repo();
        let work = inner.workdir().map(canon);
        let git = canon(inner.git_dir());
        let common = canon(inner.common_dir());
        // Committed rules only: an unreadable or empty HEAD means no rules, which only costs
        // extra scans.
        let ignore = IgnoreSet::at(self, "HEAD").ok();
        let mut roots: Vec<PathBuf> = work.iter().cloned().collect();
        for extra in [&git, &common] {
            if !roots.iter().any(|r| extra.starts_with(r)) {
                roots.push(extra.clone());
            }
        }
        let gits = vec![git, common];
        let skip = Arc::new(move |p: &Path, is_dir: bool| {
            skip(work.as_deref(), &gits, ignore.as_ref(), p, is_dir)
        });
        let cfg = NudgeConfig {
            roots,
            skip,
            debounce: DEBOUNCE,
            max_delay: MAX_DELAY,
            rescan,
            os_events,
        };
        Nudger::start(cfg, on_nudge)
    }
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// True for a path whose change cannot move the scan's answer. Ignore rules apply to folders
/// only: a tracked file can match a rule and still show as modified, and a folder is where the
/// watch cost is.
fn skip(
    work: Option<&Path>,
    gits: &[PathBuf],
    ignore: Option<&IgnoreSet>,
    p: &Path,
    is_dir: bool,
) -> bool {
    if let Some(rel) = gits.iter().find_map(|g| p.strip_prefix(g).ok()) {
        return rel
            .components()
            .next()
            .is_some_and(|c| GIT_SKIP.iter().any(|s| c.as_os_str() == *s));
    }
    let Some(Ok(rel)) = work.map(|w| p.strip_prefix(w)) else {
        return true;
    };
    if is_dir
        && p.file_name()
            .is_some_and(|n| WORK_SKIP.iter().any(|s| n == *s))
    {
        return true;
    }
    // Another repository nested inside is not ours to scan.
    if is_dir && rel.as_os_str() != "" && p.join(".git").exists() {
        return true;
    }
    let Some(rel) = rel.to_str() else {
        return false;
    };
    let rel = rel.replace('\\', "/");
    is_dir && !rel.is_empty() && ignore.is_some_and(|i| i.is_ignored_dir(&rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> IgnoreSet {
        IgnoreSet::from_files([(".gitignore".to_owned(), "dist/\n*.log\n".to_owned())])
    }

    #[test]
    fn git_internals_and_build_output_are_skipped_but_refs_and_sources_are_not() {
        let w = Some(Path::new("/w"));
        let gits = vec![PathBuf::from("/w/.git")];
        let s = |p: &str, d: bool| skip(w, &gits, Some(&rules()), Path::new(p), d);
        assert!(s("/w/.git/objects", true));
        assert!(s("/w/.git/gitterrace", true));
        assert!(!s("/w/.git/index", false));
        assert!(!s("/w/.git/HEAD", false));
        assert!(!s("/w/.git/refs/heads", true));
        assert!(!s("/w/.git/packed-refs", false));
        assert!(s("/w/target", true));
        assert!(s("/w/web/node_modules", true));
        assert!(s("/w/dist", true));
        assert!(s("/w/logs.log", true));
        assert!(!s("/w/run.log", false), "a tracked file may match a rule");
        assert!(!s("/w/src/main.rs", false));
        assert!(!s("/w/src", true));
        assert!(s("/elsewhere/x", false));
        let bare = |p: &str| skip(None, &gits, None, Path::new(p), true);
        assert!(!bare("/w/.git/refs"));
        assert!(bare("/w/.git/objects"));
    }
}
