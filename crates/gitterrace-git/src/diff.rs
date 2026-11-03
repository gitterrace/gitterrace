mod hunks;
mod stream;

use gix::bstr::ByteSlice;
use gix::diff::blob::{Algorithm, Diff, InternedInput};
use gix::object::tree::diff::ChangeDetached;

use crate::error::git;
use crate::{ChangeKind, CodeCounts, FileDiff, Hunk, Result};

pub(crate) use hunks::build_hunks;
pub(crate) use stream::{Window, stream};

/// Same probe window as git: a NUL in the first 8000 bytes marks a blob binary.
const BINARY_PROBE: usize = 8000;
/// Blob buffers a worker's diff cache keeps between commits; see `changes_with`.
const KEEP_BLOB_BUFFERS: usize = 64;

struct Counted {
    additions: u32,
    deletions: u32,
    binary: bool,
    hunks: Vec<Hunk>,
    lines: u32,
    rewritten: u32,
    code: CodeCounts,
}

/// Line counts of one blob pair, shared by every index worker. A pair's counts depend only on
/// the two blob ids (the index never asks for hunks and always uses one text limit), and the
/// same pair recurs across branches and merges: 30% of first-parent changes on git.git, 67% on
/// rust. A hit skips reading, inflating and diffing both blobs. Bounded: when full it starts
/// over, which costs only recomputation, never a different answer.
pub(crate) struct CountMemo {
    map: std::sync::Mutex<std::collections::HashMap<MemoKey, MemoCounts>>,
    cap: usize,
}

/// (old blob, new blob, rename tracking found them identical).
type MemoKey = (Option<gix::ObjectId>, Option<gix::ObjectId>, bool);

#[derive(Clone, Copy)]
struct MemoCounts {
    additions: u32,
    deletions: u32,
    binary: bool,
    lines: u32,
    rewritten: u32,
    code: CodeCounts,
}

impl CountMemo {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            map: std::sync::Mutex::default(),
            cap,
        }
    }

    fn get(&self, key: &MemoKey) -> Option<Counted> {
        let c = *self.map.lock().ok()?.get(key)?;
        Some(Counted {
            additions: c.additions,
            deletions: c.deletions,
            binary: c.binary,
            hunks: Vec::new(),
            lines: c.lines,
            rewritten: c.rewritten,
            code: c.code,
        })
    }

    fn put(&self, key: MemoKey, c: &Counted) {
        let Ok(mut map) = self.map.lock() else { return };
        if map.len() >= self.cap {
            map.clear();
        }
        map.insert(
            key,
            MemoCounts {
                additions: c.additions,
                deletions: c.deletions,
                binary: c.binary,
                lines: c.lines,
                rewritten: c.rewritten,
                code: c.code,
            },
        );
    }
}

impl Counted {
    fn empty(binary: bool) -> Self {
        Self {
            additions: 0,
            deletions: 0,
            binary,
            hunks: Vec::new(),
            lines: 0,
            rewritten: 0,
            code: CodeCounts::default(),
        }
    }
}

/// Changes of `commit` against its first parent (or the empty tree for a root).
pub(crate) fn changes(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    with_hunks: bool,
    text_limit: u64,
) -> Result<Vec<FileDiff>> {
    let mut cache = resource_cache(repo)?;
    changes_with(repo, commit, with_hunks, text_limit, &mut cache, None)
}

/// The blob cache a tree diff needs. Building it reads attributes and, in a bare clone,
/// unpacks the whole HEAD tree into an index, so a worker builds it once for many commits.
///
/// The index the attributes come from is owned here and dropped with the cache. The gix
/// shortcut keeps it in the repository's shared index snapshot, which every clone of the
/// repository holds for the life of the server: the whole HEAD index, sized by the tree.
pub(crate) fn resource_cache(repo: &gix::Repository) -> Result<gix::diff::blob::Platform> {
    let index = match repo.open_index() {
        Ok(index) => index,
        Err(_) => match repo.head_tree_id_or_empty() {
            Ok(tree) => repo.index_from_tree(&tree).map_err(git)?,
            Err(e) => return Err(git(e)),
        },
    };
    let attrs = repo
        .attributes_only(
            &index,
            gix::worktree::stack::state::attributes::Source::IdMapping,
        )
        .map_err(git)?
        .detach();
    gix::diff::resource_cache(
        repo,
        gix::diff::blob::pipeline::Mode::ToGit,
        attrs,
        gix::diff::blob::pipeline::WorktreeRoots::default(),
    )
    .map_err(git)
}

/// [`changes`] with a caller-owned blob cache, cleared here after every commit, and an optional
/// memo of counts per blob pair. The memo is only consulted without hunks, and all its users
/// must pass the same `text_limit`.
pub(crate) fn changes_with(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    with_hunks: bool,
    text_limit: u64,
    cache: &mut gix::diff::blob::Platform,
    memo: Option<&CountMemo>,
) -> Result<Vec<FileDiff>> {
    let new_tree = commit.tree().map_err(git)?;
    let old_tree = first_parent_tree(repo, commit)?;
    let empty = repo.empty_tree();
    let old = old_tree.as_ref().unwrap_or(&empty);
    let opts = gix::diff::Options::default().with_rewrites(Some(gix::diff::Rewrites::default()));
    let mut raw = Vec::new();
    let diffed = gix::diff::tree_with_rewrites(
        gix::objs::TreeRefIter::from_bytes(&old.data, old.id.kind()),
        gix::objs::TreeRefIter::from_bytes(&new_tree.data, new_tree.id.kind()),
        cache,
        &mut gix::diff::tree::State::default(),
        &repo.objects,
        |change| {
            raw.push(change.into_owned());
            Ok(std::ops::ControlFlow::Continue(()))
        },
        opts.into(),
    );
    // Keeping the buffers saves an allocation per blob on the next commit, but after a mass
    // rename the free list holds one buffer per compared blob (thousands, megabytes each) and
    // only halves per commit. Ten workers holding that cost hundreds of MB of peak memory.
    if cache.clear_resource_cache_keep_allocation() > KEEP_BLOB_BUFFERS {
        cache.clear_resource_cache();
    }
    diffed.map_err(git)?;
    let mut out = Vec::with_capacity(raw.len());
    for change in raw {
        let memo = memo.filter(|_| !with_hunks);
        if let Some(diff) = file_diff(repo, change, with_hunks, text_limit, memo)? {
            out.push(diff);
        }
    }
    Ok(out)
}

/// Tree of the first parent, `None` for a root. A shallow clone's boundary commit is a root
/// too: its parents are not in the object store, and git shows it as adding every file. Any
/// other missing parent is an error, so the indexer retries the commit instead of caching it.
fn first_parent_tree<'r>(
    repo: &'r gix::Repository,
    commit: &gix::Commit<'r>,
) -> Result<Option<gix::Tree<'r>>> {
    let Some(parent) = commit.parent_ids().next() else {
        return Ok(None);
    };
    if is_shallow_boundary(repo, commit.id) {
        return Ok(None);
    }
    let tree = parent
        .object()
        .and_then(gix::Object::peel_to_commit)
        .map_err(git)?
        .tree()
        .map_err(git)?;
    Ok(Some(tree))
}

/// Whether `id` is listed in the repository's shallow file.
pub(crate) fn is_shallow_boundary(repo: &gix::Repository, id: gix::ObjectId) -> bool {
    repo.shallow_commits()
        .ok()
        .flatten()
        .is_some_and(|ids| ids.iter().any(|s| *s == id))
}

/// The `index`th file of [`changes`] with hunks, diffing only that one file's blobs.
pub(crate) fn change_at(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    index: usize,
    text_limit: u64,
) -> Result<Option<FileDiff>> {
    let new_tree = commit.tree().map_err(git)?;
    let old_tree = first_parent_tree(repo, commit)?;
    let opts = gix::diff::Options::default().with_rewrites(Some(gix::diff::Rewrites::default()));
    let raw = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), opts)
        .map_err(git)?;
    let mut seen = 0;
    for change in raw {
        let is_tree = match &change {
            ChangeDetached::Addition { entry_mode, .. }
            | ChangeDetached::Deletion { entry_mode, .. }
            | ChangeDetached::Modification { entry_mode, .. }
            | ChangeDetached::Rewrite { entry_mode, .. } => entry_mode.is_tree(),
        };
        if is_tree {
            continue;
        }
        if seen == index {
            return file_diff(repo, change, true, text_limit, None);
        }
        seen += 1;
    }
    Ok(None)
}

/// Old and new mode of a change that altered it, e.g. a file made executable.
fn mode_change(change: &ChangeDetached) -> Option<(u16, u16)> {
    match change {
        ChangeDetached::Modification {
            previous_entry_mode,
            entry_mode,
            ..
        } => Some((previous_entry_mode.value(), entry_mode.value())),
        ChangeDetached::Rewrite {
            source_entry_mode,
            entry_mode,
            ..
        } => Some((source_entry_mode.value(), entry_mode.value())),
        _ => None,
    }
    .filter(|(a, b)| a != b)
}

fn file_diff(
    repo: &gix::Repository,
    change: ChangeDetached,
    with_hunks: bool,
    text_limit: u64,
    memo: Option<&CountMemo>,
) -> Result<Option<FileDiff>> {
    let is_tree = match &change {
        ChangeDetached::Addition { entry_mode, .. }
        | ChangeDetached::Deletion { entry_mode, .. }
        | ChangeDetached::Modification { entry_mode, .. }
        | ChangeDetached::Rewrite { entry_mode, .. } => entry_mode.is_tree(),
    };
    if is_tree {
        return Ok(None);
    }
    let rewrite_stats = match &change {
        ChangeDetached::Rewrite { diff, .. } => Some(diff.as_ref().map(|_| ())),
        _ => None,
    };
    let modes = mode_change(&change);
    let (path, old_path, kind, old_id, new_id, diffable) = match change {
        ChangeDetached::Addition {
            location,
            entry_mode,
            id,
            ..
        } => (
            location,
            None,
            ChangeKind::Added,
            None,
            Some(id),
            !entry_mode.is_commit(),
        ),
        ChangeDetached::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => (
            location,
            None,
            ChangeKind::Deleted,
            Some(id),
            None,
            !entry_mode.is_commit(),
        ),
        ChangeDetached::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => (
            location,
            None,
            ChangeKind::Modified,
            Some(previous_id),
            Some(id),
            !entry_mode.is_commit() && !previous_entry_mode.is_commit(),
        ),
        ChangeDetached::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            ..
        } => (
            location,
            Some(source_location),
            ChangeKind::Renamed,
            Some(source_id),
            Some(id),
            !entry_mode.is_commit() && !source_entry_mode.is_commit(),
        ),
    };
    let counted = counts(
        repo,
        (old_id, new_id),
        diffable,
        rewrite_stats == Some(None),
        with_hunks,
        text_limit,
        memo,
    )?;
    Ok(Some(FileDiff {
        path: path.to_str_lossy().into_owned(),
        kind,
        old_path: old_path.map(|p| p.to_str_lossy().into_owned()),
        additions: counted.additions,
        deletions: counted.deletions,
        binary: counted.binary,
        hunks: counted.hunks,
        lines: counted.lines,
        rewritten: counted.rewritten,
        code: counted.code,
        modes,
    }))
}

/// The counts of one changed file, from `memo` when it has the blob pair.
fn counts(
    repo: &gix::Repository,
    (old_id, new_id): (Option<gix::ObjectId>, Option<gix::ObjectId>),
    diffable: bool,
    pure_move: bool,
    with_hunks: bool,
    text_limit: u64,
    memo: Option<&CountMemo>,
) -> Result<Counted> {
    let key = (old_id, new_id, pure_move);
    let counted = if !diffable {
        Counted::empty(false)
    } else if let Some(hit) = memo.and_then(|m| m.get(&key)) {
        hit
    } else if too_large(repo, old_id, text_limit) || too_large(repo, new_id, text_limit) {
        // Never read, never diffed: Myers on two huge blobs costs minutes.
        Counted::empty(true)
    } else if !with_hunks && pure_move {
        // Rename tracking found identical content: a pure move, nothing changed.
        let lines = line_count(bytes(blob(repo, new_id)?.as_ref()));
        Counted {
            lines,
            ..Counted::empty(false)
        }
    } else {
        let old = blob(repo, old_id)?;
        let new = blob(repo, new_id)?;
        count(bytes(old.as_ref()), bytes(new.as_ref()), with_hunks)
    };
    if let Some(m) = memo.filter(|_| diffable) {
        m.put(key, &counted);
    }
    Ok(counted)
}

/// The blob, still attached to `repo`: its buffer goes back to the repository's free list on
/// drop, so a worker diffing thousands of commits reuses a few buffers instead of allocating
/// one per blob (the churn the allocator then keeps resident).
fn blob(repo: &gix::Repository, id: Option<gix::ObjectId>) -> Result<Option<gix::Object<'_>>> {
    id.map(|id| repo.find_object(id).map_err(git)).transpose()
}

fn bytes<'a>(object: Option<&'a gix::Object<'_>>) -> &'a [u8] {
    object.map_or(&[], |o| o.data.as_slice())
}

/// Whether the blob is over `limit` bytes, from its header alone.
fn too_large(repo: &gix::Repository, id: Option<gix::ObjectId>, limit: u64) -> bool {
    id.and_then(|id| repo.find_header(id).ok())
        .is_some_and(|h| h.size() > limit)
}

pub(crate) fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_PROBE)].contains(&0)
}

#[expect(
    clippy::naive_bytecount,
    reason = "one short pass per blob, a new dependency is not worth it"
)]
pub(crate) fn line_count(data: &[u8]) -> u32 {
    if is_binary(data) {
        return 0;
    }
    let n = data.iter().filter(|b| **b == b'\n').count()
        + usize::from(!data.is_empty() && !data.ends_with(b"\n"));
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Sum over change blocks of the smaller side: lines that replaced existing lines.
fn rewritten(diff: &Diff) -> u32 {
    diff.hunks()
        .map(|h| (h.before.end - h.before.start).min(h.after.end - h.after.start))
        .sum()
}

fn squash(line: &[u8]) -> Vec<u8> {
    line.iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect()
}

/// The counts of `diff` with whitespace ignored, like `git diff -w`, and blank lines dropped.
/// Works hunk by hunk, so the cost follows the size of the change, not of the file: the lines
/// come from the tokens `input` already holds, never from splitting the whole file again.
fn code_counts(input: &InternedInput<&[u8]>, diff: &Diff) -> CodeCounts {
    let mut out = CodeCounts::default();
    let line = |t: &gix::diff::blob::Token| input.interner[*t];
    for h in diff.hunks() {
        let before: Vec<Vec<u8>> = input.before[h.before.start as usize..h.before.end as usize]
            .iter()
            .map(|t| squash(line(t)))
            .filter(|l| !l.is_empty())
            .collect();
        let after: Vec<Vec<u8>> = input.after[h.after.start as usize..h.after.end as usize]
            .iter()
            .map(|t| squash(line(t)))
            .filter(|l| !l.is_empty())
            .collect();
        if before.is_empty() || after.is_empty() {
            out.deletions += u32::try_from(before.len()).unwrap_or(u32::MAX);
            out.additions += u32::try_from(after.len()).unwrap_or(u32::MAX);
            continue;
        }
        let mut input: InternedInput<Vec<u8>> = InternedInput::default();
        input.update_before(before.into_iter());
        input.update_after(after.into_iter());
        let sub = Diff::compute(Algorithm::Myers, &input);
        out.additions += sub.count_additions();
        out.deletions += sub.count_removals();
        out.rewritten += rewritten(&sub);
    }
    out
}

fn count(old: &[u8], new: &[u8], with_hunks: bool) -> Counted {
    if is_binary(old) || is_binary(new) {
        return Counted::empty(true);
    }
    let input = InternedInput::new(old, new);
    // Myers matches the counts of `git log --numstat`.
    let mut diff = Diff::compute(Algorithm::Myers, &input);
    let (additions, deletions) = (diff.count_additions(), diff.count_removals());
    let rewritten = rewritten(&diff);
    let code = if additions + deletions == 0 {
        CodeCounts::default()
    } else {
        code_counts(&input, &diff)
    };
    let hunks = if with_hunks {
        diff.postprocess_lines(&input);
        build_hunks(old, new, &diff)
    } else {
        Vec::new()
    };
    Counted {
        additions,
        deletions,
        binary: false,
        hunks,
        lines: line_count(new),
        rewritten,
        code,
    }
}

#[cfg(test)]
mod tests;
