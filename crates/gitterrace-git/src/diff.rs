use std::ops::Range;

use gix::bstr::ByteSlice;
use gix::diff::blob::{Algorithm, Diff, InternedInput};
use gix::object::tree::diff::ChangeDetached;

use crate::error::git;
use crate::{ChangeKind, FileDiff, Hunk, HunkLine, LineKind, Result};

const CONTEXT: usize = 3;
/// Same probe window as git: a NUL in the first 8000 bytes marks a blob binary.
const BINARY_PROBE: usize = 8000;

struct Counted {
    additions: u32,
    deletions: u32,
    binary: bool,
    hunks: Vec<Hunk>,
}

/// Changes of `commit` against its first parent (or the empty tree for a root).
pub(crate) fn changes(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    with_hunks: bool,
) -> Result<Vec<FileDiff>> {
    let new_tree = commit.tree().map_err(git)?;
    let old_tree = match commit.parent_ids().next() {
        Some(parent) => Some(
            parent
                .object()
                .and_then(gix::Object::peel_to_commit)
                .map_err(git)?
                .tree()
                .map_err(git)?,
        ),
        None => None,
    };
    let opts = gix::diff::Options::default().with_rewrites(Some(gix::diff::Rewrites::default()));
    let raw = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), opts)
        .map_err(git)?;
    let mut out = Vec::with_capacity(raw.len());
    for change in raw {
        if let Some(diff) = file_diff(repo, change, with_hunks)? {
            out.push(diff);
        }
    }
    Ok(out)
}

fn file_diff(
    repo: &gix::Repository,
    change: ChangeDetached,
    with_hunks: bool,
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
        ChangeDetached::Rewrite { diff, .. } => Some(*diff),
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
    let counted = if !diffable {
        Counted {
            additions: 0,
            deletions: 0,
            binary: false,
            hunks: Vec::new(),
        }
    } else if let (false, Some(stats)) = (with_hunks, rewrite_stats) {
        // Rename tracking already diffed the pair; `None` means identical content.
        let (additions, deletions) = stats.map_or((0, 0), |s| (s.insertions, s.removals));
        Counted {
            additions,
            deletions,
            binary: false,
            hunks: Vec::new(),
        }
    } else {
        let old = blob(repo, old_id)?;
        let new = blob(repo, new_id)?;
        count(&old, &new, with_hunks)
    };
    Ok(Some(FileDiff {
        path: path.to_str_lossy().into_owned(),
        kind,
        old_path: old_path.map(|p| p.to_str_lossy().into_owned()),
        additions: counted.additions,
        deletions: counted.deletions,
        binary: counted.binary,
        hunks: counted.hunks,
    }))
}

fn blob(repo: &gix::Repository, id: Option<gix::ObjectId>) -> Result<Vec<u8>> {
    match id {
        Some(id) => Ok(repo.find_object(id).map_err(git)?.detach().data),
        None => Ok(Vec::new()),
    }
}

fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_PROBE)].contains(&0)
}

fn count(old: &[u8], new: &[u8], with_hunks: bool) -> Counted {
    if is_binary(old) || is_binary(new) {
        return Counted {
            additions: 0,
            deletions: 0,
            binary: true,
            hunks: Vec::new(),
        };
    }
    let input = InternedInput::new(old, new);
    // Myers matches the counts of `git log --numstat`.
    let mut diff = Diff::compute(Algorithm::Myers, &input);
    let (additions, deletions) = (diff.count_additions(), diff.count_removals());
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

fn build_hunks(old: &[u8], new: &[u8], diff: &Diff) -> Vec<Hunk> {
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
