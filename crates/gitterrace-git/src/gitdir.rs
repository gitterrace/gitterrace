//! State that lives in the git directory rather than in commits: HEAD state, linked worktrees,
//! branch divergence, annotated tags and stashes.
//!
//! Linked worktrees are described by `.git/worktrees/<id>/`, whose `gitdir` file names a path
//! anywhere on disk. Anyone who can write the repository controls that text, so this module never
//! opens, stats or canonicalises it: every file read goes through [`read_inside`], which refuses
//! anything outside the common git directory.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use gix::bstr::ByteSlice;

use crate::error::git;
use crate::{Repo, Result};

/// HEAD, `locked` and `gitdir` are a few bytes. Anything bigger is not git's and is skipped.
const MAX_META_BYTES: u64 = 4096;

/// An operation git is in the middle of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InProgress {
    Rebase,
    Merge,
    CherryPick,
    Revert,
    Bisect,
    ApplyMailbox,
}

/// Where HEAD points and what is half done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadState {
    /// Branch HEAD points at, `None` when detached.
    pub branch: Option<String>,
    /// Commit HEAD resolves to, `None` on an unborn branch.
    pub commit: Option<String>,
    pub in_progress: Option<InProgress>,
}

/// One linked worktree, read from the common git directory only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedWorktree {
    pub id: String,
    /// The `gitdir` text as written, for display only. Never opened.
    pub path: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub locked: bool,
}

/// A local branch against the base branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchStatus {
    pub name: String,
    pub target: String,
    pub ahead: usize,
    pub behind: usize,
}

impl BranchStatus {
    /// Every commit of the branch is already in the base.
    pub fn merged(&self) -> bool {
        self.ahead == 0
    }
}

/// The message and tagger of an annotated tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagAnnotation {
    pub tagger: Option<crate::Signature>,
    pub subject: String,
    pub body: String,
}

/// One `stash@{n}` entry, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    pub index: usize,
    pub commit: String,
    pub message: String,
    pub time: i64,
}

impl Repo {
    /// HEAD's branch or detached commit, and any operation in progress.
    pub fn head_state(&self) -> HeadState {
        let repo = self.repo();
        let in_progress = repo.state().map(|s| {
            use gix::state::InProgress as G;
            match s {
                G::Rebase | G::RebaseInteractive | G::ApplyMailboxRebase => InProgress::Rebase,
                G::Merge => InProgress::Merge,
                G::CherryPick | G::CherryPickSequence => InProgress::CherryPick,
                G::Revert | G::RevertSequence => InProgress::Revert,
                G::Bisect => InProgress::Bisect,
                G::ApplyMailbox => InProgress::ApplyMailbox,
            }
        });
        HeadState {
            branch: self.head_branch(),
            commit: repo.head_id().ok().map(|id| id.to_string()),
            in_progress,
        }
    }

    /// Linked worktrees, sorted by id. Entries that are symlinks, oversized or unreadable are
    /// skipped rather than followed.
    pub fn linked_worktrees(&self) -> Result<Vec<LinkedWorktree>> {
        let repo = self.repo();
        let common = repo.common_dir().to_path_buf();
        let Ok(entries) = fs::read_dir(common.join("worktrees")) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(head) = read_inside(&common, &["worktrees", &id, "HEAD"]) else {
                continue;
            };
            let (branch, commit) = parse_head(&repo, head.trim());
            out.push(LinkedWorktree {
                path: read_inside(&common, &["worktrees", &id, "gitdir"])
                    .map(|p| p.trim().trim_end_matches("/.git").to_owned()),
                locked: read_inside(&common, &["worktrees", &id, "locked"]).is_some(),
                id,
                branch,
                commit,
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Ahead and behind counts of every local branch against `base`. Results are cached per
    /// (tip, base) pair, so only branches that moved are walked again.
    pub fn branch_status(&self, base: &str) -> Result<Vec<BranchStatus>> {
        let repo = self.repo();
        let base_id = Self::resolve_id(&repo, base)?;
        let mut out = Vec::new();
        for r in self.refs()? {
            if r.kind != crate::RefKind::Branch {
                continue;
            }
            let tip = gix::ObjectId::from_hex(r.target.as_bytes()).map_err(git)?;
            let cached = self
                .divergence
                .lock()
                .ok()
                .and_then(|c| c.get(&(tip, base_id)).copied());
            let (ahead, behind) = if let Some(v) = cached {
                v
            } else {
                let v = (
                    count_only(&repo, tip, base_id)?,
                    count_only(&repo, base_id, tip)?,
                );
                if let Ok(mut c) = self.divergence.lock() {
                    c.insert((tip, base_id), v);
                }
                v
            };
            out.push(BranchStatus {
                name: r.name,
                target: r.target,
                ahead,
                behind,
            });
        }
        Ok(out)
    }

    /// Stash entries from the `refs/stash` reflog, newest first.
    pub fn stashes(&self) -> Result<Vec<Stash>> {
        let repo = self.repo();
        let Ok(Some(reference)) = repo.try_find_reference("refs/stash") else {
            return Ok(Vec::new());
        };
        let mut platform = reference.log_iter();
        let Some(lines) = platform.rev().map_err(git)? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for line in lines {
            let Ok(line) = line else { continue };
            out.push(Stash {
                index: out.len(),
                commit: line.new_oid.to_string(),
                message: line.message.to_str_lossy().into_owned(),
                time: line.signature.time.seconds,
            });
        }
        Ok(out)
    }
}

/// Message and tagger of the tag object `id`, `None` for a lightweight tag.
pub(crate) fn annotation(repo: &gix::Repository, id: gix::ObjectId) -> Option<TagAnnotation> {
    let object = repo.find_object(id).ok()?;
    let tag = object.try_into_tag().ok()?;
    let decoded = tag.decode().ok()?;
    let message = decoded.message.to_str_lossy();
    let (subject, body) = message.split_once('\n').unwrap_or((&message, ""));
    let map = gix::mailmap::Snapshot::default();
    Some(TagAnnotation {
        tagger: decoded
            .tagger()
            .ok()
            .flatten()
            .map(|s| crate::ident::signature(&map, s)),
        subject: subject.trim().to_owned(),
        body: body.trim().to_owned(),
    })
}

/// Commits reachable from `tip` and not from `hidden`.
fn count_only(repo: &gix::Repository, tip: gix::ObjectId, hidden: gix::ObjectId) -> Result<usize> {
    if tip == hidden {
        return Ok(0);
    }
    let walk = repo
        .rev_walk([tip])
        .with_hidden([hidden])
        .all()
        .map_err(git)?;
    let mut n = 0;
    for info in walk {
        info.map_err(git)?;
        n += 1;
    }
    Ok(n)
}

/// A worktree HEAD file: `ref: refs/heads/x` or a bare commit id.
fn parse_head(repo: &gix::Repository, head: &str) -> (Option<String>, Option<String>) {
    if let Some(name) = head.strip_prefix("ref: ") {
        let commit = repo
            .try_find_reference(name)
            .ok()
            .flatten()
            .and_then(|mut r| r.peel_to_id().ok())
            .map(|id| id.to_string());
        let short = name.strip_prefix("refs/heads/").unwrap_or(name).to_owned();
        return (Some(short), commit);
    }
    let commit = gix::ObjectId::from_hex(head.as_bytes())
        .ok()
        .map(|id| id.to_string());
    (None, commit)
}

/// The one door to files under the common git directory. Each part must be a single plain
/// component, no component may be a symlink, and the file must be small. Anything else reads as
/// absent, so a hostile entry can neither point the reader elsewhere nor fail the page.
fn read_inside(common: &Path, parts: &[&str]) -> Option<String> {
    let mut path = PathBuf::from(common);
    for part in parts {
        let mut comps = Path::new(part).components();
        if !matches!(
            (comps.next(), comps.next()),
            (Some(Component::Normal(_)), None)
        ) {
            return None;
        }
        path.push(part);
        if fs::symlink_metadata(&path).ok()?.file_type().is_symlink() {
            return None;
        }
    }
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_META_BYTES {
        return None;
    }
    let mut text = String::new();
    fs::File::open(&path)
        .ok()?
        .take(MAX_META_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;
    use std::fs;
    use std::path::Path;

    use crate::{InProgress, Repo};

    fn commit(dir: &Path, file: &str, body: &str) {
        fs::write(dir.join(file), body).expect("write");
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", file]);
    }

    fn base() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = tmp.path().join("repo");
        fs::create_dir(&d).expect("mkdir");
        git(&d, &["init", "-q"]);
        commit(&d, "a", "1\n");
        (tmp, d)
    }

    #[test]
    fn divergence_and_merged() {
        let (_t, d) = base();
        git(&d, &["checkout", "-qb", "side"]);
        commit(&d, "b", "1\n");
        commit(&d, "c", "1\n");
        git(&d, &["checkout", "-q", "main"]);
        commit(&d, "d", "1\n");
        git(&d, &["branch", "old", "main~1"]);
        let repo = Repo::discover(&d).expect("open");
        let s = repo.branch_status("main").expect("status");
        let get = |n: &str| s.iter().find(|b| b.name == n).expect(n).clone();
        assert_eq!((get("side").ahead, get("side").behind), (2, 1));
        assert!(get("old").merged() && get("old").behind == 1);
        assert_eq!((get("main").ahead, get("main").behind), (0, 0));
        // Second call answers from the cache with the same result.
        assert_eq!(repo.branch_status("main").expect("again"), s);
    }

    #[test]
    fn head_state_reports_detached_rebase() {
        let (_t, d) = base();
        git(&d, &["checkout", "-qb", "side"]);
        commit(&d, "a", "side\n");
        git(&d, &["checkout", "-q", "main"]);
        commit(&d, "a", "main\n");
        let repo = Repo::discover(&d).expect("open");
        assert_eq!(repo.head_state().branch.as_deref(), Some("main"));
        assert_eq!(repo.head_state().in_progress, None);
        assert!(
            gitterrace_testkit::try_git(&d, &["rebase", "side"]).is_err(),
            "rebase must stop on the conflict"
        );
        let repo = Repo::discover(&d).expect("reopen");
        let s = repo.head_state();
        assert_eq!(s.branch, None);
        assert_eq!(s.in_progress, Some(InProgress::Rebase));
        assert!(s.commit.is_some());
    }

    #[test]
    fn annotated_tag_and_stash() {
        let (_t, d) = base();
        git(&d, &["tag", "-a", "v1", "-m", "Release one\n\nbody text"]);
        git(&d, &["tag", "light"]);
        fs::write(d.join("a"), "dirty\n").expect("write");
        git(&d, &["stash", "push", "-qm", "wip one"]);
        let repo = Repo::discover(&d).expect("open");
        let refs = repo.refs().expect("refs");
        let v1 = refs.iter().find(|r| r.name == "v1").expect("v1");
        let note = v1.annotation.as_ref().expect("annotated");
        assert_eq!(
            (note.subject.as_str(), note.body.as_str()),
            ("Release one", "body text")
        );
        assert_eq!(note.tagger.as_ref().map(|t| t.name.as_str()), Some("T"));
        let light = refs.iter().find(|r| r.name == "light").expect("light");
        assert!(light.annotation.is_none());
        let st = repo.stashes().expect("stashes");
        assert_eq!(st.len(), 1);
        assert!(st[0].message.contains("wip one"), "{}", st[0].message);
        assert_eq!(repo.resolve(&st[0].commit).ok(), Some(st[0].commit.clone()));
    }

    #[test]
    fn linked_worktrees_listed() {
        let (t, d) = base();
        git(&d, &["branch", "side"]);
        let wt = t.path().join("wt");
        git(
            &d,
            &["worktree", "add", "-q", wt.to_str().expect("utf8"), "side"],
        );
        git(
            &d,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                t.path().join("det").to_str().expect("utf8"),
            ],
        );
        git(&d, &["worktree", "lock", "det"]);
        let repo = Repo::discover(&d).expect("open");
        let w = repo.linked_worktrees().expect("list");
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].id, "det");
        assert!(w[0].locked && w[0].branch.is_none() && w[0].commit.is_some());
        assert_eq!(w[1].branch.as_deref(), Some("side"));
        assert!(w[1].path.as_deref().is_some_and(|p| p.ends_with("/wt")));
    }

    /// Timing on a real repository: `GITTERRACE_BENCH_REPO=<path> cargo test --release -p
    /// gitterrace-git bench_branch_status -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs GITTERRACE_BENCH_REPO"]
    fn bench_branch_status() {
        let Ok(path) = std::env::var("GITTERRACE_BENCH_REPO") else {
            return;
        };
        let repo = Repo::discover(Path::new(&path)).expect("open");
        let t = std::time::Instant::now();
        let s = repo.branch_status("HEAD").expect("status");
        let cold = t.elapsed();
        let t = std::time::Instant::now();
        repo.branch_status("HEAD").expect("status");
        let warm = t.elapsed();
        let t = std::time::Instant::now();
        let w = repo.linked_worktrees().expect("worktrees");
        let wt = t.elapsed();
        let t = std::time::Instant::now();
        let h = repo.head_state();
        let hs = t.elapsed();
        println!(
            "branches={} cold={cold:?} warm={warm:?} worktrees={} in {wt:?} head={hs:?} {h:?}",
            s.len(),
            w.len()
        );
    }

    /// A hostile `.git/worktrees` must not make the reader leave the git directory: a symlinked
    /// entry, a symlinked HEAD and a `gitdir` naming `/etc` are all visible attacks.
    #[test]
    fn hostile_worktree_entries_never_read_outside() {
        let (t, d) = base();
        let id = git(&d, &["rev-parse", "HEAD"]);
        let outside = t.path().join("outside");
        fs::create_dir(&outside).expect("mkdir");
        fs::write(outside.join("HEAD"), format!("{id}\n")).expect("write");
        fs::write(outside.join("gitdir"), "/secret\n").expect("write");
        let wts = d.join(".git/worktrees");
        fs::create_dir_all(wts.join("headlink")).expect("mkdir");
        std::os::unix::fs::symlink(&outside, wts.join("dirlink")).expect("symlink");
        std::os::unix::fs::symlink(outside.join("HEAD"), wts.join("headlink/HEAD"))
            .expect("symlink");
        fs::create_dir_all(wts.join("etc")).expect("mkdir");
        fs::write(wts.join("etc/HEAD"), format!("{id}\n")).expect("write");
        fs::write(wts.join("etc/gitdir"), "/etc\n").expect("write");
        std::os::unix::fs::symlink(outside.join("gitdir"), wts.join("etc/locked"))
            .expect("symlink");
        let repo = Repo::discover(&d).expect("open");
        let w = repo.linked_worktrees().expect("list");
        let ids: Vec<&str> = w.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["etc"], "symlinked entries must be skipped");
        assert_eq!(w[0].path.as_deref(), Some("/etc"));
        assert!(!w[0].locked, "a symlinked locked file must read as absent");
        assert!(super::read_inside(&d.join(".git"), &["worktrees", "..", "HEAD"]).is_none());
        assert!(super::read_inside(&d.join(".git"), &["worktrees/etc", "HEAD"]).is_none());
    }
}
