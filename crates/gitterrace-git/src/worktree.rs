//! The Uncommitted view: HEAD against the working tree (staged, unstaged and untracked).
//!
//! This is the only module that reads the worktree. It never writes: no index refresh, no lock
//! files. Files may change or vanish while we read, so every per-file failure skips that file.

use std::collections::BTreeSet;
use std::path::Path;

use gix::bstr::ByteSlice;
use gix::diff::blob::{Algorithm, Diff, InternedInput};

use crate::error::git;
use crate::{ChangeKind, FileDiff, Hunk, HunkLine, LineKind, Repo, Result};

/// Untracked trees can be enormous (a forgotten `node_modules`), so the view stops here.
const MAX_FILES: usize = 2000;
/// Larger files are listed but shown without hunks.
const MAX_DIFF_BYTES: usize = 1024 * 1024;
const CONTEXT: usize = 3;
const BINARY_PROBE: usize = 8000;

fn open(repo: &Repo) -> Result<gix::Repository> {
    gix::open(repo.git_dir()).map_err(git)
}

/// Paths that differ between HEAD, the index and the worktree, sorted.
fn changed_paths(repo: &gix::Repository, limit: usize) -> Result<BTreeSet<String>> {
    use gix::status::{UntrackedFiles, index_worktree::Item as Iw};

    let mut out = BTreeSet::new();
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(git)?
        .untracked_files(UntrackedFiles::Files)
        .into_index_worktree_iter(Vec::new())
        .map_err(git);
    // Staged changes come from a separate tree-to-index pass below.
    for item in iter? {
        let Ok(item) = item else { continue };
        match item {
            Iw::Modification { rela_path, .. } => {
                out.insert(rela_path.to_str_lossy().into_owned());
            }
            Iw::DirectoryContents { entry, .. } => {
                if entry.status == gix::dir::entry::Status::Untracked {
                    out.insert(entry.rela_path.to_str_lossy().into_owned());
                }
            }
            Iw::Rewrite {
                source,
                dirwalk_entry,
                ..
            } => {
                out.insert(source.rela_path().to_str_lossy().into_owned());
                out.insert(dirwalk_entry.rela_path.to_str_lossy().into_owned());
            }
        }
        if out.len() >= limit {
            return Ok(out);
        }
    }
    staged_paths(repo, &mut out)?;
    Ok(out)
}

fn staged_paths(repo: &gix::Repository, out: &mut BTreeSet<String>) -> Result<()> {
    let head_tree = repo.head_tree_id_or_empty().map_err(git)?;
    let index = repo.index_or_empty().map_err(git)?;
    repo.tree_index_status(
        &head_tree,
        &index,
        None,
        gix::status::tree_index::TrackRenames::Disabled,
        |change, _, _| {
            out.insert(change.location().to_str_lossy().into_owned());
            Ok(std::ops::ControlFlow::Continue(()))
        },
    )
    .map_err(git)?;
    Ok(())
}

/// True when anything differs from HEAD. Stops at the first difference.
#[must_use]
pub fn is_dirty(repo: &Repo) -> bool {
    open(repo)
        .and_then(|r| changed_paths(&r, 1))
        .is_ok_and(|p| !p.is_empty())
}

/// Every uncommitted file as a diff against HEAD. Renames are shown as delete plus add.
pub fn worktree_diff(repo: &Repo) -> Result<Vec<FileDiff>> {
    let r = open(repo)?;
    let paths = changed_paths(&r, MAX_FILES)?;
    let work = r.workdir().map(Path::to_path_buf);
    let head = r.head_commit().ok().and_then(|c| c.tree().ok());
    let mut out = Vec::new();
    for path in paths {
        let old = head.as_ref().and_then(|t| head_blob(&r, t, &path));
        let new = work
            .as_deref()
            .map_or(Read::Skip, |w| read_worktree(w, &path));
        let new = match new {
            Read::Data(d) => Some(d),
            Read::Missing => None,
            Read::Skip => continue,
        };
        let kind = match (&old, &new) {
            (None, None) => continue,
            (None, Some(_)) => ChangeKind::Added,
            (Some(_), None) => ChangeKind::Deleted,
            (Some(a), Some(b)) if a == b => continue,
            _ => ChangeKind::Modified,
        };
        let (a, b) = (old.unwrap_or_default(), new.unwrap_or_default());
        out.push(file_diff(path, kind, &a, &b));
    }
    Ok(out)
}

fn head_blob(repo: &gix::Repository, tree: &gix::Tree<'_>, path: &str) -> Option<Vec<u8>> {
    let entry = tree.lookup_entry_by_path(path).ok()??;
    let mode = entry.mode();
    if mode.is_tree() || mode.is_commit() {
        return None;
    }
    Some(repo.find_object(entry.object_id()).ok()?.detach().data)
}

enum Read {
    Data(Vec<u8>),
    /// The file is gone, which for a tracked path means deleted.
    Missing,
    /// Unreadable, a directory or a nested repository.
    Skip,
}

fn read_worktree(work: &Path, rel: &str) -> Read {
    let full = work.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        return Read::Missing;
    };
    if meta.is_symlink() {
        return match std::fs::read_link(&full) {
            Ok(t) => Read::Data(t.to_string_lossy().into_owned().into_bytes()),
            Err(_) => Read::Skip,
        };
    }
    if !meta.is_file() {
        return Read::Skip;
    }
    match std::fs::read(&full) {
        Ok(d) => Read::Data(d),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(_) => Read::Skip,
    }
}

fn file_diff(path: String, kind: ChangeKind, old: &[u8], new: &[u8]) -> FileDiff {
    let probe = |d: &[u8]| d[..d.len().min(BINARY_PROBE)].contains(&0);
    let big = old.len() > MAX_DIFF_BYTES || new.len() > MAX_DIFF_BYTES;
    if probe(old) || probe(new) || big {
        return FileDiff {
            path,
            kind,
            old_path: None,
            additions: 0,
            deletions: 0,
            binary: true,
            hunks: Vec::new(),
        };
    }
    let input = InternedInput::new(old, new);
    let mut diff = Diff::compute(Algorithm::Myers, &input);
    let (additions, deletions) = (diff.count_additions(), diff.count_removals());
    diff.postprocess_lines(&input);
    FileDiff {
        path,
        kind,
        old_path: None,
        additions,
        deletions,
        binary: false,
        hunks: hunks(old, new, &diff),
    }
}

fn split(data: &[u8]) -> Vec<&[u8]> {
    data.split_inclusive(|b| *b == b'\n').collect()
}

fn text(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    line.to_str_lossy().into_owned()
}

/// Groups changes whose context windows touch, as `git diff -U3` does.
fn hunks(old: &[u8], new: &[u8], diff: &Diff) -> Vec<Hunk> {
    let (ol, nl) = (split(old), split(new));
    let changes: Vec<_> = diff
        .hunks()
        .map(|h| {
            (
                h.before.start as usize..h.before.end as usize,
                h.after.start as usize..h.after.end as usize,
            )
        })
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < changes.len() {
        let mut j = i;
        while j + 1 < changes.len() && changes[j + 1].0.start - changes[j].0.end <= 2 * CONTEXT {
            j += 1;
        }
        out.push(group(&changes[i..=j], &ol, &nl));
        i = j + 1;
    }
    out
}

fn ctx(old: &[&[u8]], ob: &mut usize, nb: &mut usize, n: usize, out: &mut Vec<HunkLine>) {
    for _ in 0..n {
        out.push(HunkLine {
            kind: LineKind::Context,
            old_no: u32::try_from(*ob + 1).ok(),
            new_no: u32::try_from(*nb + 1).ok(),
            text: text(old[*ob]),
        });
        *ob += 1;
        *nb += 1;
    }
}

type Change = (std::ops::Range<usize>, std::ops::Range<usize>);

fn group(changes: &[Change], old: &[&[u8]], new: &[&[u8]]) -> Hunk {
    let lead = changes[0].0.start.min(CONTEXT);
    let mut ob = changes[0].0.start - lead;
    let mut nb = changes[0].1.start - lead;
    let (old_start, new_start) = (ob, nb);
    let mut lines = Vec::new();
    for (before, after) in changes {
        let gap = before.start - ob;
        ctx(old, &mut ob, &mut nb, gap, &mut lines);
        for l in &old[before.clone()] {
            lines.push(HunkLine {
                kind: LineKind::Removed,
                old_no: u32::try_from(ob + 1).ok(),
                new_no: None,
                text: text(l),
            });
            ob += 1;
        }
        for l in &new[after.clone()] {
            lines.push(HunkLine {
                kind: LineKind::Added,
                old_no: None,
                new_no: u32::try_from(nb + 1).ok(),
                text: text(l),
            });
            nb += 1;
        }
    }
    let trail = (old.len() - ob).min(CONTEXT);
    ctx(old, &mut ob, &mut nb, trail, &mut lines);
    let (ol, nl) = (ob - old_start, nb - new_start);
    let from = |s: usize, len: usize| if len == 0 { s } else { s + 1 };
    Hunk {
        header: format!(
            "@@ -{},{ol} +{},{nl} @@",
            from(old_start, ol),
            from(new_start, nl)
        ),
        lines,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::process::Command;

    use crate::{ChangeKind, Repo, worktree_diff};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@e.x"])
            .args(args)
            .current_dir(dir)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn shows_staged_unstaged_untracked_and_deleted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        for (n, c) in [("a.txt", "1\n2\n3\n"), ("b.txt", "x\n"), ("c.txt", "y\n")] {
            std::fs::write(p.join(n), c).expect("write");
        }
        git(p, &["add", "."]);
        git(p, &["commit", "-qm", "init"]);
        let repo = Repo::discover(p).expect("repo");
        assert!(!crate::is_dirty(&repo));
        std::fs::write(p.join("a.txt"), "1\ntwo\n3\n").expect("write");
        std::fs::write(p.join("b.txt"), "staged\n").expect("write");
        git(p, &["add", "b.txt"]);
        std::fs::remove_file(p.join("c.txt")).expect("rm");
        std::fs::write(p.join("new.txt"), "n\n").expect("write");
        assert!(crate::is_dirty(&repo));
        let d = worktree_diff(&repo).expect("diff");
        let kind = |n: &str| d.iter().find(|f| f.path == n).map(|f| f.kind);
        assert_eq!(kind("a.txt"), Some(ChangeKind::Modified));
        assert_eq!(kind("b.txt"), Some(ChangeKind::Modified));
        assert_eq!(kind("c.txt"), Some(ChangeKind::Deleted));
        assert_eq!(kind("new.txt"), Some(ChangeKind::Added));
        let a = d.iter().find(|f| f.path == "a.txt").expect("a");
        assert_eq!((a.additions, a.deletions), (1, 1));
        assert!(!p.join(".git/index.lock").exists());
    }
}
