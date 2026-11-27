//! The Uncommitted view: HEAD against the working tree (staged, unstaged and untracked).
//!
//! This is the only module that reads the worktree. It never writes: no index refresh, no lock
//! files. Files may change or vanish while we read, so every per-file failure skips that file.

mod beneath;
#[cfg(test)]
mod limit_tests;
mod pair;
mod snapshot;
pub use snapshot::{
    BIG_HASH, SinceDiff, SnapFile, Snapshot, content_id, worktree_diff_since, worktree_snapshot,
};

use std::collections::BTreeSet;
use std::path::Path;

use gix::bstr::ByteSlice;
use gix::diff::blob::{Algorithm, Diff, InternedInput};

use crate::error::git;
use crate::worktree::beneath::{open_beneath, read_bounded};
use crate::{Blob, ChangeKind, FileDiff, Repo, Result, WorktreeScope};

/// Changed files the view lists by default (`limits.worktree_files`): untracked trees can be
/// enormous (a forgotten `node_modules`), so the view stops there.
pub const WORKTREE_FILES: usize = 2000;
/// Largest file diffed by default (`limits.worktree_diff_kb`); larger ones are listed without
/// hunks.
pub const WORKTREE_DIFF_BYTES: u64 = 1024 * 1024;

fn open(repo: &Repo) -> Result<gix::Repository> {
    gix::open(repo.git_dir()).map_err(git)
}

/// How many files each [`WorktreeScope`] lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScopeCounts {
    pub all: usize,
    pub staged: usize,
    pub unstaged: usize,
}

/// The paths of one scope, sorted, stopping at `limit`.
fn scoped_paths(
    repo: &gix::Repository,
    scope: WorktreeScope,
    limit: usize,
) -> Result<BTreeSet<String>> {
    scoped_listing(repo, scope, limit).map(|l| l.0)
}

/// [`scoped_paths`] and whether the list may be incomplete: the scan reached `limit`, so more
/// changed files may exist than it names. Conservative on purpose: a list of exactly `limit`
/// files reads as possibly incomplete, because proving otherwise would walk the whole tree.
fn scoped_listing(
    repo: &gix::Repository,
    scope: WorktreeScope,
    limit: usize,
) -> Result<(BTreeSet<String>, bool)> {
    use gix::status::{UntrackedFiles, index_worktree::Item as Iw};

    let mut out = BTreeSet::new();
    // A bare repository has no working tree to differ from HEAD, and diffing HEAD against its
    // empty index would walk the whole tree on every watcher tick.
    if repo.workdir().is_none() {
        return Ok((out, false));
    }
    if scope == WorktreeScope::Staged {
        staged_paths(repo, &mut out, limit)?;
        let cut = out.len() >= limit;
        return Ok((out, cut));
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
            Iw::Rewrite { source, dirwalk_entry, .. } => {
                out.insert(source.rela_path().to_str_lossy().into_owned());
                out.insert(dirwalk_entry.rela_path.to_str_lossy().into_owned());
            }
        }
        if out.len() >= limit {
            return Ok((out, true));
        }
    }
    if scope == WorktreeScope::All {
        staged_paths(repo, &mut out, limit)?;
    }
    let cut = out.len() >= limit;
    Ok((out, cut))
}

/// The file count of every scope: the rows its table lists, so a tab never disagrees with the
/// page under it. One function counts them all, after renames are paired.
pub fn worktree_counts(repo: &Repo) -> Result<ScopeCounts> {
    let rows = |scope| worktree_diff_scoped(repo, scope).map(|f| f.len());
    Ok(ScopeCounts {
        all: rows(WorktreeScope::All)?,
        staged: rows(WorktreeScope::Staged)?,
        unstaged: rows(WorktreeScope::Unstaged)?,
    })
}

/// Adds the paths staged against HEAD to `out`, stopping once it lists `limit` files: a huge
/// staged change must stay as cheap as a huge untracked tree.
///
/// Exact renames are tracked, so a `git mv` lists both of its paths but counts as one file:
/// counting paths would cut the deleted sides (they sort after the new names) off a big move
/// and leave `pair::exact_moves` nothing to pair.
fn staged_paths(repo: &gix::Repository, out: &mut BTreeSet<String>, limit: usize) -> Result<()> {
    use gix::diff::index::ChangeRef;

    let head_tree = repo.head_tree_id_or_empty().map_err(git)?;
    let index = repo.index_or_empty().map_err(git)?;
    let exact = gix::diff::Rewrites { percentage: None, limit: 0, ..Default::default() };
    let mut pairs = 0;
    repo.tree_index_status(
        &head_tree,
        &index,
        None,
        gix::status::tree_index::TrackRenames::Given(exact),
        |change, _, _| {
            if let ChangeRef::Rewrite { source_location, .. } = &change
                && out.insert(source_location.to_str_lossy().into_owned())
            {
                pairs += 1;
            }
            out.insert(change.location().to_str_lossy().into_owned());
            Ok(if out.len() - pairs >= limit {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            })
        },
    )
    .map_err(git)?;
    Ok(())
}

/// A hash of the unstaged and untracked state.
///
/// Which paths differ from the index and their size and mtime, so editing a modified file again
/// moves it too. Reads only, like the rest of this module. Capped like the view, so a forgotten
/// `node_modules` stays cheap.
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
        if items.len() >= repo.worktree_files() {
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
    open(repo).and_then(|r| scoped_paths(&r, WorktreeScope::All, 1)).is_ok_and(|p| !p.is_empty())
}

/// Every uncommitted file as a diff against HEAD.
pub fn worktree_diff(repo: &Repo) -> Result<Vec<FileDiff>> {
    worktree_diff_scoped(repo, WorktreeScope::All)
}

/// The files of one [`WorktreeScope`] as diffs: HEAD..index for `Staged`, index..worktree for
/// `Unstaged` (untracked files are additions), HEAD..worktree for `All`.
///
/// Only `All` pairs exact renames, see `pair::exact_moves`: it reads HEAD and the disk, so it
/// would mislabel a half-staged move. Files are read and diffed one at a time, so the bytes
/// held at once are one file's old and new side, not the whole change.
pub fn worktree_diff_scoped(repo: &Repo, scope: WorktreeScope) -> Result<Vec<FileDiff>> {
    worktree_listed(repo, scope).map(|l| l.files)
}

/// The files of one [`WorktreeScope`] and whether the scan stopped at `limits.worktree_files`.
#[derive(Debug, Default)]
pub struct Listed {
    pub files: Vec<FileDiff>,
    /// The list may be incomplete: the scan reached the limit. This says nothing about diff
    /// lines, which are cut separately.
    pub files_omitted: bool,
}

/// [`worktree_diff_scoped`] with the completeness of the file list.
pub fn worktree_listed(repo: &Repo, scope: WorktreeScope) -> Result<Listed> {
    let r = open(repo)?;
    let big = repo.worktree_diff_bytes();
    let (paths, files_omitted) = scoped_listing(&r, scope, repo.worktree_files())?;
    let head = r.head_commit().ok().and_then(|c| c.tree().ok());
    let index = r.index_or_empty().map_err(git)?;
    let root = r.workdir().and_then(|w| w.canonicalize().ok());
    let moves = if scope == WorktreeScope::All {
        pair::exact_moves(&r, head.as_ref(), root.as_deref(), &paths, big)
    } else {
        pair::Moves::default()
    };
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
        let (old, new) = sides(&r, scope, head.as_ref(), &index, root.as_deref(), path, big);
        out.extend(classify(path, &old, &new));
    }
    Ok(Listed { files: out, files_omitted })
}

/// The two sides of `path` that `scope` compares.
fn sides(
    r: &gix::Repository,
    scope: WorktreeScope,
    head: Option<&gix::Tree<'_>>,
    index: &gix::index::File,
    root: Option<&Path>,
    path: &str,
    big: u64,
) -> (Read, Read) {
    let from_head = || head.map_or(Read::Missing, |t| head_blob(r, t, path, big));
    let from_index = || index_blob(r, index, path, big);
    let from_disk = || root.map_or(Read::Skip, |w| read_worktree(w, path, big));
    match scope {
        WorktreeScope::All => (from_head(), from_disk()),
        WorktreeScope::Staged => (from_head(), from_index()),
        WorktreeScope::Unstaged => (from_index(), from_disk()),
    }
}

/// The working-tree bytes of one changed file, for the image previews of the working tree page.
///
/// The same guard as [`worktree_file_diff`]: `None` unless `path` is one of the changed files,
/// so an ignored file, anything under `.git` or a path with `..` is never read. Also `None` for
/// a deleted file and one over `max` bytes, which is never read.
pub fn worktree_bytes(repo: &Repo, path: &str, max: u64) -> Result<Option<Vec<u8>>> {
    if !is_plain(path) {
        return Ok(None);
    }
    let r = open(repo)?;
    if !scoped_paths(&r, WorktreeScope::All, repo.worktree_files())?.contains(path) {
        return Ok(None);
    }
    let Some(root) = r.workdir().and_then(|w| w.canonicalize().ok()) else {
        return Ok(None);
    };
    // `Read` counts held bytes on drop in tests, so the bytes are copied out, not moved.
    let read = read_worktree(&root, path, max);
    Ok(matches!(read, Read::Data(_)).then(|| read.bytes().to_vec()))
}

/// What the working tree holds at one path, against HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WtFile {
    /// Not one of the changed files: it reads the same as HEAD (or is not ours to read).
    Same,
    /// A changed file that is gone from the disk.
    Gone,
    /// A changed file's bytes.
    Data(Vec<u8>),
    /// A changed file over the limit: never read.
    Big,
}

/// The working tree read once for many paths: the set of changed files is listed a single
/// time, so asking about each path of a basket does not walk the tree again.
pub struct WorktreeView {
    changed: BTreeSet<String>,
    root: Option<std::path::PathBuf>,
}

impl WorktreeView {
    /// Lists the changed files. A bare repository has none.
    pub fn open(repo: &Repo) -> Result<Self> {
        let r = open(repo)?;
        let changed = scoped_paths(&r, WorktreeScope::All, repo.worktree_files())?;
        let root = r.workdir().and_then(|w| w.canonicalize().ok());
        Ok(Self { changed, root })
    }

    /// Whether there is a working tree to read.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        self.root.is_some()
    }

    /// `path` as the disk holds it, read only when it is a changed file and at most `max` bytes.
    /// The same guard as [`worktree_bytes`]: an ignored file, anything under `.git` or a path
    /// with `..` is never read.
    #[must_use]
    pub fn file(&self, path: &str, max: u64) -> WtFile {
        let Some(root) = self.root.as_deref().filter(|_| is_plain(path)) else {
            return WtFile::Same;
        };
        if !self.changed.contains(path) {
            return WtFile::Same;
        }
        match &read_worktree(root, path, max) {
            Read::Data(d) => WtFile::Data(d.clone()),
            Read::Big => WtFile::Big,
            Read::Missing => WtFile::Gone,
            Read::Skip => WtFile::Same,
        }
    }
}

/// One changed file as it is on disk, for the working tree's blob view: the same guard and
/// `max` as [`worktree_bytes`], with the text decoded under the repository's text limit.
pub fn worktree_blob(repo: &Repo, path: &str, max: u64) -> Result<Option<(Blob, Vec<u8>)>> {
    let Some(bytes) = worktree_bytes(repo, path, max)? else {
        return Ok(None);
    };
    let blob = crate::repo::blob_of(path, &bytes, repo.text_limit());
    Ok(Some((blob, bytes)))
}

/// One uncommitted file against HEAD: the body a folded file of the working tree page loads.
///
/// Whether every component of `path` is a real name: no empty, `.`, `..` or `.git` part.
fn is_plain(path: &str) -> bool {
    path.split('/')
        .all(|c| !c.is_empty() && c != "." && c != ".." && !c.eq_ignore_ascii_case(".git"))
}

/// `None` unless `path` is one of the page's changed files, so an ignored file, anything
/// under `.git` or a path with `..` is never read, whatever the URL asks for.
pub fn worktree_file_diff(repo: &Repo, path: &str) -> Result<Option<FileDiff>> {
    worktree_file_diff_scoped(repo, path, WorktreeScope::All)
}

/// [`worktree_file_diff`] for one [`WorktreeScope`]: `None` unless `path` is changed in that scope.
pub fn worktree_file_diff_scoped(
    repo: &Repo,
    path: &str,
    scope: WorktreeScope,
) -> Result<Option<FileDiff>> {
    if !is_plain(path) {
        return Ok(None);
    }
    let r = open(repo)?;
    let big = repo.worktree_diff_bytes();
    if !scoped_paths(&r, scope, repo.worktree_files())?.contains(path) {
        return Ok(None);
    }
    let head = r.head_commit().ok().and_then(|c| c.tree().ok());
    let index = r.index_or_empty().map_err(git)?;
    let root = r.workdir().and_then(|w| w.canonicalize().ok());
    let (old, new) = sides(&r, scope, head.as_ref(), &index, root.as_deref(), path, big);
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
    Some(file_diff(path.to_owned(), kind, old.bytes(), new.bytes(), big))
}

fn head_blob(repo: &gix::Repository, tree: &gix::Tree<'_>, path: &str, big: u64) -> Read {
    let Ok(Some(entry)) = crate::treepath::lookup(tree, path) else {
        return Read::Missing;
    };
    let mode = entry.mode();
    if mode.is_tree() || mode.is_commit() {
        return Read::Missing;
    }
    blob_read(repo, entry.object_id(), big)
}

fn index_blob(repo: &gix::Repository, index: &gix::index::File, path: &str, big: u64) -> Read {
    let Some(e) = index.entry_by_path(path.into()) else {
        return Read::Missing;
    };
    if e.mode.is_submodule() || e.mode.is_sparse() {
        return Read::Missing;
    }
    blob_read(repo, e.id, big)
}

fn blob_read(repo: &gix::Repository, id: gix::ObjectId, big: u64) -> Read {
    match repo.find_header(id) {
        Ok(h) if h.size() > big => Read::Big,
        Ok(_) => repo.find_object(id).map_or(Read::Skip, |o| Read::data(o.detach().data)),
        Err(_) => Read::Skip,
    }
}

enum Read {
    Data(Vec<u8>),
    /// Over the diff size limit: listed without hunks, never read.
    Big,
    /// The file is gone, which for a tracked path means deleted.
    Missing,
    /// Unreadable, a directory, a nested repository or behind a symlinked directory.
    Skip,
}

impl Read {
    #[cfg_attr(not(test), expect(clippy::missing_const_for_fn, reason = "counts bytes in tests"))]
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
fn read_worktree(work: &Path, rel: &str, big: u64) -> Read {
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
        return std::fs::read_link(&full)
            .map_or(Read::Skip, |t| Read::data(t.to_string_lossy().into_owned().into_bytes()));
    }
    if !meta.is_file() {
        return Read::Skip;
    }
    if meta.len() > big {
        return Read::Big;
    }
    match open_beneath(work, rel).and_then(|f| read_bounded(&f, big)) {
        Ok(Some(d)) => Read::data(d),
        // Grew past the limit after the size check above.
        Ok(None) => Read::Big,
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
mod tests;
