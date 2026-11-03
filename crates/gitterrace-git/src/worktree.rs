//! The Uncommitted view: HEAD against the working tree (staged, unstaged and untracked).
//!
//! This is the only module that reads the worktree. It never writes: no index refresh, no lock
//! files. Files may change or vanish while we read, so every per-file failure skips that file.

mod pair;

use std::collections::BTreeSet;
use std::path::Path;

use gix::bstr::ByteSlice;
use gix::diff::blob::{Algorithm, Diff, InternedInput};

use crate::error::git;
use crate::{ChangeKind, FileDiff, Repo, Result};

/// Untracked trees can be enormous (a forgotten `node_modules`), so the view stops here.
const MAX_FILES: usize = 2000;
/// Larger files are listed but shown without hunks.
const MAX_DIFF_BYTES: usize = 1024 * 1024;

fn open(repo: &Repo) -> Result<gix::Repository> {
    gix::open(repo.git_dir()).map_err(git)
}

/// Paths that differ between HEAD, the index and the worktree, sorted.
fn changed_paths(repo: &gix::Repository, limit: usize) -> Result<BTreeSet<String>> {
    use gix::status::{UntrackedFiles, index_worktree::Item as Iw};

    let mut out = BTreeSet::new();
    // A bare repository has no working tree to differ from HEAD, and diffing HEAD against its
    // empty index would walk the whole tree on every watcher tick.
    if repo.workdir().is_none() {
        return Ok(out);
    }
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

/// A hash of the unstaged and untracked state.
///
/// Which paths differ from the index and their size and mtime, so editing a modified file again moves it too. Reads only, like the rest of this
/// module. Capped like the view, so a forgotten `node_modules` stays cheap.
pub fn worktree_signature(repo: &Repo) -> Result<u64> {
    use gix::status::{UntrackedFiles, index_worktree::Item as Iw};
    use std::hash::{Hash, Hasher};

    let r = open(repo)?;
    let work = r.workdir().map(Path::to_path_buf);
    if work.is_none() {
        return Ok(0);
    }
    let mut items: Vec<(String, u64, u128)> = Vec::new();
    let iter = r
        .status(gix::progress::Discard)
        .map_err(git)?
        .untracked_files(UntrackedFiles::Files)
        .into_index_worktree_iter(Vec::new())
        .map_err(git)?;
    for item in iter {
        let Ok(item) = item else { continue };
        let path = match item {
            Iw::Modification { rela_path, .. } => rela_path.to_str_lossy().into_owned(),
            Iw::DirectoryContents { entry, .. } => {
                if entry.status != gix::dir::entry::Status::Untracked {
                    continue;
                }
                entry.rela_path.to_str_lossy().into_owned()
            }
            Iw::Rewrite { dirwalk_entry, .. } => {
                dirwalk_entry.rela_path.to_str_lossy().into_owned()
            }
        };
        let (size, mtime) = work
            .as_ref()
            .and_then(|w| std::fs::symlink_metadata(w.join(&path)).ok())
            .map_or((0, 0), |m| {
                let t = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                (m.len(), t)
            });
        items.push((path, size, mtime));
        if items.len() >= MAX_FILES {
            break;
        }
    }
    items.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    items.hash(&mut h);
    Ok(h.finish())
}

/// True when anything differs from HEAD. Stops at the first difference.
#[must_use]
pub fn is_dirty(repo: &Repo) -> bool {
    open(repo)
        .and_then(|r| changed_paths(&r, 1))
        .is_ok_and(|p| !p.is_empty())
}

/// Every uncommitted file as a diff against HEAD.
///
/// Only exact renames are paired, see `pair::exact_moves`. Files are read and diffed one at a
/// time, so the bytes held at once are one file's old and new side, not the whole change.
pub fn worktree_diff(repo: &Repo) -> Result<Vec<FileDiff>> {
    let r = open(repo)?;
    let paths = changed_paths(&r, MAX_FILES)?;
    let head = r.head_commit().ok().and_then(|c| c.tree().ok());
    let root = r.workdir().and_then(|w| w.canonicalize().ok());
    let moves = pair::exact_moves(&r, head.as_ref(), root.as_deref(), &paths);
    let mut out = Vec::new();
    for path in &paths {
        if moves.sources.contains(path.as_str()) {
            continue;
        }
        if let Some(from) = moves.to.get(path.as_str()) {
            out.push(FileDiff {
                kind: ChangeKind::Renamed,
                old_path: Some(from.clone()),
                ..file_diff(path.clone(), ChangeKind::Renamed, &[], &[], false)
            });
            continue;
        }
        let old = head
            .as_ref()
            .map_or(Read::Missing, |t| head_blob(&r, t, path));
        let new = root
            .as_deref()
            .map_or(Read::Skip, |w| read_worktree(w, path));
        out.extend(classify(path, &old, &new));
    }
    Ok(out)
}

/// One uncommitted file against HEAD: the body a folded file of the working tree page loads.
///
/// `None` unless `path` is one of the page's changed files, so an ignored file, anything
/// under `.git` or a path with `..` is never read, whatever the URL asks for.
pub fn worktree_file_diff(repo: &Repo, path: &str) -> Result<Option<FileDiff>> {
    let plain = path
        .split('/')
        .all(|c| !c.is_empty() && c != "." && c != ".." && !c.eq_ignore_ascii_case(".git"));
    if !plain {
        return Ok(None);
    }
    let r = open(repo)?;
    if !changed_paths(&r, MAX_FILES)?.contains(path) {
        return Ok(None);
    }
    let head = r.head_commit().ok().and_then(|c| c.tree().ok());
    let root = r.workdir().and_then(|w| w.canonicalize().ok());
    let old = head
        .as_ref()
        .map_or(Read::Missing, |t| head_blob(&r, t, path));
    let new = root
        .as_deref()
        .map_or(Read::Skip, |w| read_worktree(w, path));
    Ok(classify(path, &old, &new))
}

fn classify(path: &str, old: &Read, new: &Read) -> Option<FileDiff> {
    let kind = match (old, new) {
        (Read::Skip, _) | (_, Read::Skip) | (Read::Missing, Read::Missing) => return None,
        (Read::Missing, _) => ChangeKind::Added,
        (_, Read::Missing) => ChangeKind::Deleted,
        (Read::Data(a), Read::Data(b)) if a == b => return None,
        _ => ChangeKind::Modified,
    };
    let big = matches!(old, Read::Big) || matches!(new, Read::Big);
    Some(file_diff(
        path.to_owned(),
        kind,
        old.bytes(),
        new.bytes(),
        big,
    ))
}

fn head_blob(repo: &gix::Repository, tree: &gix::Tree<'_>, path: &str) -> Read {
    let Ok(Some(entry)) = crate::treepath::lookup(tree, path) else {
        return Read::Missing;
    };
    let mode = entry.mode();
    if mode.is_tree() || mode.is_commit() {
        return Read::Missing;
    }
    let id = entry.object_id();
    match repo.find_header(id) {
        Ok(h) if h.size() > MAX_DIFF_BYTES as u64 => Read::Big,
        Ok(_) => repo
            .find_object(id)
            .map_or(Read::Skip, |o| Read::data(o.detach().data)),
        Err(_) => Read::Skip,
    }
}

enum Read {
    Data(Vec<u8>),
    /// Over [`MAX_DIFF_BYTES`]: listed without hunks, never read.
    Big,
    /// The file is gone, which for a tracked path means deleted.
    Missing,
    /// Unreadable, a directory, a nested repository or behind a symlinked directory.
    Skip,
}

impl Read {
    #[cfg_attr(
        not(test),
        expect(clippy::missing_const_for_fn, reason = "counts bytes in tests")
    )]
    fn data(d: Vec<u8>) -> Self {
        #[cfg(test)]
        held::add(d.len());
        Self::Data(d)
    }

    fn bytes(&self) -> &[u8] {
        match self {
            Self::Data(d) => d,
            Self::Big | Self::Missing | Self::Skip => &[],
        }
    }
}

#[cfg(test)]
impl Drop for Read {
    fn drop(&mut self) {
        if let Self::Data(d) = self {
            held::sub(d.len());
        }
    }
}

/// Test-only count of file bytes alive at once, and its peak.
#[cfg(test)]
mod held {
    use std::cell::Cell;

    thread_local! {
        static NOW: Cell<usize> = const { Cell::new(0) };
        static PEAK: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn add(n: usize) {
        let now = NOW.get() + n;
        NOW.set(now);
        PEAK.set(PEAK.get().max(now));
    }

    pub(super) fn sub(n: usize) {
        NOW.set(NOW.get().saturating_sub(n));
    }

    /// Resets the peak and returns the one before.
    pub(super) fn take_peak() -> usize {
        PEAK.replace(NOW.get())
    }
}

/// `work` must be canonical. A path whose parent resolves outside it (a symlinked directory)
/// is skipped, so the view never renders files from elsewhere on disk.
fn read_worktree(work: &Path, rel: &str) -> Read {
    let full = work.join(rel);
    match full.parent().map(Path::canonicalize) {
        Some(Ok(parent)) if parent.starts_with(work) => {}
        Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Read::Missing,
        _ => return Read::Skip,
    }
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        return Read::Missing;
    };
    if meta.is_symlink() {
        return std::fs::read_link(&full).map_or(Read::Skip, |t| {
            Read::data(t.to_string_lossy().into_owned().into_bytes())
        });
    }
    if !meta.is_file() {
        return Read::Skip;
    }
    if meta.len() > MAX_DIFF_BYTES as u64 {
        return Read::Big;
    }
    match std::fs::read(&full) {
        Ok(d) => Read::data(d),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(_) => Read::Skip,
    }
}

fn file_diff(path: String, kind: ChangeKind, old: &[u8], new: &[u8], big: bool) -> FileDiff {
    if crate::diff::is_binary(old) || crate::diff::is_binary(new) || big {
        return FileDiff {
            path,
            kind,
            old_path: None,
            additions: 0,
            deletions: 0,
            binary: true,
            hunks: Vec::new(),
            lines: 0,
            rewritten: 0,
            code: crate::CodeCounts::default(),
            modes: None,
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
        hunks: crate::diff::build_hunks(old, new, &diff),
        lines: 0,
        rewritten: 0,
        code: crate::CodeCounts::default(),
        modes: None,
    }
}

#[cfg(test)]
mod pair_tests;
#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;

    use crate::{ChangeKind, Repo, worktree_diff};

    #[test]
    fn a_bare_repository_is_never_dirty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        std::fs::create_dir(&src).expect("mkdir");
        git(&src, &["init", "-q", "-b", "main"]);
        std::fs::write(src.join("a.txt"), "1\n").expect("write");
        git(&src, &["add", "."]);
        git(&src, &["commit", "-qm", "init"]);
        let bare = dir.path().join("bare.git");
        git(
            dir.path(),
            &["clone", "-q", "--bare", "src", bare.to_str().expect("utf8")],
        );
        let repo = Repo::discover(&bare).expect("repo");
        assert!(!crate::is_dirty(&repo));
        assert_eq!(crate::worktree_signature(&repo).expect("sig"), 0);
        assert!(worktree_diff(&repo).expect("diff").is_empty());
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

    /// `GITTERRACE_BENCH_REPO=<path> cargo test --release -p gitterrace-git -- --ignored --nocapture signature_cost`.
    #[test]
    #[ignore = "measurement, run by hand in release"]
    #[expect(clippy::print_stdout, reason = "a bench reports its measurement")]
    fn signature_cost() {
        let p = std::env::var("GITTERRACE_BENCH_REPO").expect("GITTERRACE_BENCH_REPO");
        let repo = Repo::discover(std::path::Path::new(&p)).expect("repo");
        crate::worktree_signature(&repo).expect("warm");
        let t = std::time::Instant::now();
        for _ in 0..20 {
            crate::worktree_signature(&repo).expect("sig");
        }
        println!("worktree signature: {:?} per tick", t.elapsed() / 20);
    }

    #[test]
    fn signature_moves_on_edit_and_new_file_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "1\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-qm", "init"]);
        let repo = Repo::discover(p).expect("repo");
        let sig = || crate::worktree_signature(&repo).expect("sig");
        let clean = sig();
        assert_eq!(sig(), clean, "an untouched tree is stable");
        std::fs::write(p.join("a.txt"), "changed\n").expect("write");
        let edited = sig();
        assert_ne!(edited, clean, "editing a tracked file");
        std::fs::write(p.join("a.txt"), "changed again\n").expect("write");
        let again = sig();
        assert_ne!(again, edited, "editing a modified file again");
        std::fs::write(p.join("new.txt"), "n\n").expect("write");
        assert_ne!(sig(), again, "a new untracked file");
    }

    #[test]
    #[cfg(unix)]
    fn large_file_listed_without_reading_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "a\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-qm", "init"]);
        // Unreadable, so any read fails and the file would be skipped: listing it proves
        // the size cap ran on metadata alone.
        let big = p.join("dump.sql");
        std::fs::File::create(&big)
            .and_then(|f| f.set_len(2 * super::MAX_DIFF_BYTES as u64))
            .expect("sparse");
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        let d = worktree_diff(&Repo::discover(p).expect("repo")).expect("diff");
        let f = d.iter().find(|f| f.path == "dump.sql").expect("listed");
        assert!(f.binary && f.hunks.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_directory_is_not_followed() {
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::write(outside.path().join("secret.txt"), "secret\n").expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::create_dir(p.join("docs")).expect("dir");
        std::fs::write(p.join("docs/secret.txt"), "public\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-qm", "init"]);
        std::fs::remove_dir_all(p.join("docs")).expect("rm");
        std::os::unix::fs::symlink(outside.path(), p.join("docs")).expect("link");
        let d = worktree_diff(&Repo::discover(p).expect("repo")).expect("diff");
        let leaked = d
            .iter()
            .flat_map(|f| &f.hunks)
            .flat_map(|h| &h.lines)
            .any(|l| l.text.contains("secret"));
        assert!(!leaked, "{d:?}");
    }
}
