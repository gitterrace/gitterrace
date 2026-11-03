use std::ops::Range;

use gix::bstr::ByteSlice;
use gix::diff::blob::{Algorithm, Diff, InternedInput};
use gix::object::tree::diff::ChangeDetached;

use crate::error::git;
use crate::{ChangeKind, CodeCounts, FileDiff, Hunk, HunkLine, LineKind, Result};

const CONTEXT: usize = 3;
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

fn lines(data: &[u8]) -> Vec<&[u8]> {
    data.split_inclusive(|b| *b == b'\n').collect()
}

fn text(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    line.to_str_lossy().into_owned()
}

pub(crate) fn build_hunks(old: &[u8], new: &[u8], diff: &Diff) -> Vec<Hunk> {
    let (old_lines, new_lines) = (lines(old), lines(new));
    let changes: Vec<(Range<usize>, Range<usize>)> = diff
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
        out.push(group(&changes[i..=j], &old_lines, &new_lines));
        i = j + 1;
    }
    out
}

fn context(old: &[&[u8]], ob: &mut usize, nb: &mut usize, n: usize, lines: &mut Vec<HunkLine>) {
    for _ in 0..n {
        lines.push(HunkLine {
            kind: LineKind::Context,
            old_no: u32::try_from(*ob + 1).ok(),
            new_no: u32::try_from(*nb + 1).ok(),
            text: text(old[*ob]),
        });
        *ob += 1;
        *nb += 1;
    }
}

fn group(changes: &[(Range<usize>, Range<usize>)], old: &[&[u8]], new: &[&[u8]]) -> Hunk {
    let lead = changes[0].0.start.min(CONTEXT);
    let mut ob = changes[0].0.start - lead;
    let mut nb = changes[0].1.start - lead;
    let (old_start, new_start) = (ob, nb);
    let mut lines = Vec::new();
    for (before, after) in changes {
        let gap = before.start - ob;
        context(old, &mut ob, &mut nb, gap, &mut lines);
        for line in &old[before.clone()] {
            lines.push(HunkLine {
                kind: LineKind::Removed,
                old_no: u32::try_from(ob + 1).ok(),
                new_no: None,
                text: text(line),
            });
            ob += 1;
        }
        for line in &new[after.clone()] {
            lines.push(HunkLine {
                kind: LineKind::Added,
                old_no: None,
                new_no: u32::try_from(nb + 1).ok(),
                text: text(line),
            });
            nb += 1;
        }
    }
    let trail = (old.len() - ob).min(CONTEXT);
    context(old, &mut ob, &mut nb, trail, &mut lines);
    let (old_len, new_len) = (ob - old_start, nb - new_start);
    // An empty side is addressed by the line before it, as git prints it.
    let from = |start: usize, len: usize| if len == 0 { start } else { start + 1 };
    Hunk {
        header: format!(
            "@@ -{},{} +{},{} @@",
            from(old_start, old_len),
            old_len,
            from(new_start, new_len),
            new_len
        ),
        lines,
    }
}

/// How far [`stream`]'s workers may run ahead of the oldest result not yet emitted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Window {
    /// Items started past the next one to emit.
    pub(crate) items: usize,
    /// Estimated bytes of finished results held for reordering.
    pub(crate) bytes: usize,
}

/// What the workers of [`stream`] wait on.
#[derive(Default)]
struct Ahead {
    next: usize,
    held: usize,
}

/// Runs `work` over `items` on `workers` threads that each take the next item when free, and
/// hands each result to `emit` in the order of `items` as soon as it and every earlier one are
/// done. `setup` builds one worker's state (a repository handle, a blob cache) once. Commit cost
/// is very uneven: one mass rename can take seconds while its neighbours take microseconds, so
/// a fixed split per thread, or a barrier per batch, would leave the other cores idle behind
/// the slowest share. Here a slow item holds back only the emitting, never the free workers. A worker may run at most `window` items ahead of the next one to
/// emit, so results waiting in the reorder buffer stay bounded however slow one item is.
/// The window is also bounded in bytes: `weight` estimates a result's size, and while the
/// results held exceed `budget` only the item next to emit may start. So a run of huge results
/// (a merge window's worth of large merges) cannot hold `window` times their size.
/// The first error (from a worker or from `emit`) stops the run and is returned.
pub(crate) fn stream<T: Sync, S, R: Send>(
    items: &[T],
    workers: usize,
    window: Window,
    setup: impl Fn() -> Result<S> + Sync,
    work: impl Fn(&mut S, &T) -> Result<R> + Sync,
    weight: impl Fn(&R) -> usize + Sync,
    mut emit: impl FnMut(usize, R) -> Result<()>,
) -> Result<()> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex, mpsc};

    let Window {
        items: span,
        bytes: budget,
    } = window;
    let span = span.max(1);
    let workers = workers.clamp(1, items.len().max(1));
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    // The next item to emit and the bytes of finished results not yet emitted; workers wait
    // on them to stay inside the window.
    let emitted = (Mutex::new(Ahead::default()), Condvar::new());
    // A worker counts its result's bytes before sending it, so results still in the channel
    // count too; the channel itself never blocks a worker.
    let (tx, rx) = mpsc::channel::<(usize, Result<R>, usize)>();
    let release = |next: usize, freed: usize| {
        if let Ok(mut e) = emitted.0.lock() {
            e.next = next;
            e.held = e.held.saturating_sub(freed);
        }
        emitted.1.notify_all();
    };
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (next, stop, emitted, setup, work, weight) =
                (&next, &stop, &emitted, &setup, &work, &weight);
            scope.spawn(move || {
                let mut state = match setup() {
                    Ok(s) => s,
                    Err(e) => {
                        drop(tx.send((usize::MAX, Err(e), 0)));
                        return;
                    }
                };
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { return };
                    let Ok(mut e) = emitted.0.lock() else { return };
                    // The item next to emit always runs, or a full budget would wait forever.
                    while i > e.next
                        && (i >= e.next.saturating_add(span) || e.held > budget)
                        && !stop.load(Ordering::Relaxed)
                    {
                        let Ok(g) = emitted.1.wait(e) else { return };
                        e = g;
                    }
                    drop(e);
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let r = work(&mut state, item);
                    let w = r.as_ref().map_or(0, weight);
                    if let Ok(mut e) = emitted.0.lock() {
                        e.held = e.held.saturating_add(w);
                    }
                    if tx.send((i, r, w)).is_err() {
                        return;
                    }
                }
            });
        }
        drop(tx);
        let mut held = std::collections::BTreeMap::new();
        let mut want = 0;
        let mut out = Ok(());
        while want < items.len() {
            let Ok((i, r, w)) = rx.recv() else {
                out = Err(crate::Error::Git("diff worker panicked".to_string()));
                break;
            };
            held.insert(i, (r, w));
            let before = want;
            let mut freed = 0_usize;
            while let Some((r, w)) = held.remove(&want) {
                freed = freed.saturating_add(w);
                if let Err(e) = r.and_then(|r| emit(want, r)) {
                    out = Err(e);
                    break;
                }
                want += 1;
            }
            if out.is_err() {
                break;
            }
            // A worker that could not start reports under usize::MAX.
            if let Some((Err(e), _)) = held.remove(&usize::MAX) {
                out = Err(e);
                break;
            }
            if want != before {
                release(want, freed);
            }
        }
        if out.is_err() {
            stop.store(true, Ordering::Relaxed);
            release(usize::MAX, 0);
        }
        drop(rx);
        out
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::diff::{Window, stream};

    fn collect<R: Send>(
        items: &[u64],
        workers: usize,
        window: usize,
        work: impl Fn(&u64) -> crate::Result<R> + Sync,
    ) -> crate::Result<Vec<R>> {
        let mut out = Vec::new();
        stream(
            items,
            workers,
            Window {
                items: window,
                bytes: usize::MAX,
            },
            || Ok(()),
            |(), i| work(i),
            |_| 0,
            |n, r| {
                assert_eq!(n, out.len(), "emitted out of order");
                out.push(r);
                Ok(())
            },
        )?;
        Ok(out)
    }

    /// One slow item must not hold back the items queued behind it: the others drain on free
    /// threads, so the whole run takes about the slowest item, not the slowest item plus its
    /// share. A fixed split runs items 1..25 after item 0 on one thread (0.98 s), and a
    /// barrier per batch of 25 waits 0.5 s on the first batch (1.0 s in all); a shared queue
    /// spreads them over the other three (about 0.66 s).
    #[test]
    fn one_slow_item_does_not_serialize_the_rest() -> crate::Result<()> {
        let items: Vec<u64> = (0..100).collect();
        let t0 = Instant::now();
        let out = collect(&items, 4, 100, |i| {
            std::thread::sleep(Duration::from_millis(if *i == 0 { 500 } else { 20 }));
            Ok(i * 2)
        })?;
        let took = t0.elapsed();
        assert_eq!(out, items.iter().map(|i| i * 2).collect::<Vec<_>>());
        assert!(took < Duration::from_millis(880), "took {took:?}");
        Ok(())
    }

    /// Workers never run more than `window` items ahead of the oldest one not yet emitted, so
    /// the results held for reordering behind a slow item stay bounded.
    #[test]
    fn workers_stay_inside_the_window() -> crate::Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let items: Vec<u64> = (0..400).collect();
        let started = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        collect(&items, 8, 16, |i| {
            started.fetch_add(1, Ordering::SeqCst);
            if *i == 0 {
                std::thread::sleep(Duration::from_millis(200));
                peak.store(started.load(Ordering::SeqCst), Ordering::SeqCst);
            }
            Ok(())
        })?;
        assert!(
            peak.load(Ordering::SeqCst) <= 16,
            "{}",
            peak.load(Ordering::SeqCst)
        );
        Ok(())
    }

    /// While the held results exceed the byte budget, only the item next to emit may start,
    /// so a run of huge results behind a slow item cannot fill the whole (wide) count window.
    /// Catches a budget that is never checked, or one checked only against the count.
    #[test]
    fn held_results_stay_inside_the_byte_budget() -> crate::Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let items: Vec<u64> = (0..400).collect();
        let started = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        stream(
            &items,
            8,
            Window {
                items: 1000,
                bytes: 10 * 100,
            },
            || Ok(()),
            |(), i| {
                started.fetch_add(1, Ordering::SeqCst);
                if *i == 0 {
                    std::thread::sleep(Duration::from_millis(200));
                    peak.store(started.load(Ordering::SeqCst), Ordering::SeqCst);
                }
                Ok(())
            },
            |()| 100,
            |_, ()| Ok(()),
        )?;
        // Budget 10 results, plus one per worker that passed the check before it filled.
        let peak = peak.load(Ordering::SeqCst);
        assert!(peak <= 10 + 1 + 8, "{peak} started behind the slow item");
        Ok(())
    }

    #[test]
    fn results_keep_input_order_and_errors_surface() {
        let items: Vec<u64> = (0..1000).collect();
        assert!(collect(&items, 7, 32, |i| Ok(*i)).is_ok_and(|o| o == items));
        let err = collect(&items, 3, 32, |i| {
            if *i == 500 {
                Err(crate::Error::Git("boom".into()))
            } else {
                Ok(())
            }
        });
        assert!(err.is_err());
        let failed_emit = stream(
            &items,
            4,
            Window {
                items: 8,
                bytes: usize::MAX,
            },
            || Ok(()),
            |(), _| Ok(()),
            |()| 0,
            |n, ()| {
                if n == 10 {
                    Err(crate::Error::Git("disk".into()))
                } else {
                    Ok(())
                }
            },
        );
        assert!(failed_emit.is_err());
        let failed_setup = stream(
            &items,
            4,
            Window {
                items: 8,
                bytes: usize::MAX,
            },
            || Err::<(), _>(crate::Error::Git("x".into())),
            |(), _| Ok(()),
            |()| 0,
            |_, ()| Ok(()),
        );
        assert!(failed_setup.is_err());
        assert!(collect(&[], 4, 8, |_| Ok(())).is_ok_and(|o: Vec<()>| o.is_empty()));
    }

    /// Every commit diffed with the blob-pair memo (shared, and so small it starts over many
    /// times) has the same file stats as without it. Catches a memo key that misses an input
    /// the counts depend on: a pure move and an edit of the same pair, an addition and a
    /// deletion of one blob, or a hit returning another pair's counts.
    #[test]
    fn the_count_memo_never_changes_a_result() -> crate::Result<()> {
        let dir = tempfile::tempdir().map_err(crate::error::git)?;
        let mut f = gitterrace_testkit::Fixture::init(dir.path());
        let body = (0..30).fold(String::new(), |b, l| b + &format!("line {l}\n"));
        for round in 0..4 {
            f = f
                .write("a.txt", &body)
                .write("b.txt", &format!("{body}round {round}\n"))
                .commit("same blobs again");
            f.git(&["mv", "a.txt", "moved.txt"]);
            f = f.commit("pure move");
            f.git(&["rm", "-q", "moved.txt"]);
            f = f.write("c.txt", " spaced  \n\n").commit("delete and add");
            // One new blob reached from two different old blobs, in two files.
            f = f
                .write("x.txt", "one\ntwo\n")
                .write("y.txt", "one\nthree\nfour\n")
                .commit("two starts");
            f = f
                .write("x.txt", "one\nend\n")
                .write("y.txt", "one\nend\n")
                .commit("one end");
        }
        let repo = crate::Repo::discover(dir.path())?;
        let gix = repo.repo();
        let memo = super::CountMemo::new(3);
        let mut cache = super::resource_cache(&gix)?;
        for id in repo.rev_list("HEAD")? {
            let commit = gix
                .find_commit(repo.full_id(&gix, &id)?)
                .map_err(crate::error::git)?;
            let plain = super::changes(&gix, &commit, false, crate::MAX_TEXT_BYTES)?;
            for _ in 0..2 {
                let memoized = super::changes_with(
                    &gix,
                    &commit,
                    false,
                    crate::MAX_TEXT_BYTES,
                    &mut cache,
                    Some(&memo),
                )?;
                assert_eq!(format!("{memoized:?}"), format!("{plain:?}"), "commit {id}");
            }
        }
        Ok(())
    }

    /// The cache built from an owned index reads the same attributes as the gix shortcut, so
    /// every diff of a history with `binary`, `diff=`, `-diff` and `text` rules, renames and
    /// edits across them is the same. Red when the attributes are dropped (an empty index).
    #[test]
    fn the_owned_index_cache_diffs_as_the_shared_one() -> crate::Result<()> {
        let dir = tempfile::tempdir().map_err(crate::error::git)?;
        let mut f = gitterrace_testkit::Fixture::init(dir.path());
        let body = (0..40).fold(String::new(), |b, l| b + &format!("line {l}\n"));
        f = f
            .write(
                ".gitattributes",
                "*.dat binary\n*.md diff=markdown\n*.gen -diff\n*.txt text eol=crlf\n",
            )
            .write("a.dat", &body)
            .write("b.md", &body)
            .write("c.gen", &body)
            .write("d.txt", &body)
            .commit("attributes");
        // Attributes reach the result through rename similarity: a `binary` or `-diff` file
        // moved with an edit is no rename to compare by content, a text one is.
        f.git(&["mv", "a.dat", "moved.dat"]);
        f.git(&["mv", "c.gen", "moved.gen"]);
        f.git(&["mv", "b.md", "moved.md"]);
        f = f
            .write("moved.dat", &format!("{body}more\n"))
            .write("moved.gen", &format!("{body}more\n"))
            .write("moved.md", &format!("{body}more\n"))
            .write("d.txt", &format!("x\n{body}"))
            .commit("renames and edits");
        f.git(&["mv", "moved.gen", "e.txt"]);
        let _done = f
            .write("e.txt", &format!("{body}again\n"))
            .commit("across rules");
        let repo = crate::Repo::discover(dir.path())?;
        let gix = repo.repo();
        let mut owned = super::resource_cache(&gix)?;
        let mut shared = gix
            .diff_resource_cache_for_tree_diff()
            .map_err(crate::error::git)?;
        for id in repo.rev_list("HEAD")? {
            let commit = gix
                .find_commit(repo.full_id(&gix, &id)?)
                .map_err(crate::error::git)?;
            for hunks in [false, true] {
                let a = super::changes_with(&gix, &commit, hunks, 1 << 20, &mut owned, None)?;
                let b = super::changes_with(&gix, &commit, hunks, 1 << 20, &mut shared, None)?;
                assert_eq!(format!("{a:?}"), format!("{b:?}"), "commit {id}");
            }
        }
        Ok(())
    }
}
