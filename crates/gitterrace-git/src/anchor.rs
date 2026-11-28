//! Where the code a finding points at is now. Worked out on every read, never stored, so a
//! finding keeps what the user chose and the status keeps saying what became of the code.
//!
//! The finding names a commit, a path and maybe lines. The anchor reads that text at the
//! commit, then at HEAD (following a rename), then in the working tree when the file is one of
//! the changed files. The lines are looked for as the same block of text: found where they were
//! or moved, they are `untouched`. Otherwise the old text is diffed against the new and the
//! range is mapped through the hunks: a range with a surviving or replaced line is
//! `lines edited` at its mapped span, one whose lines are all gone is `lines changed`. When line
//! anchoring is not
//! possible (binary, too large, a range past the end) the detail is dropped to the file and
//! never guessed. Nothing here writes.

use std::collections::HashMap;

use gitterrace_types::AnchorStatus;
use gix::diff::blob::{Algorithm, Diff, InternedInput};

use crate::worktree::{WorktreeView, WtFile};
use crate::{Repo, Result};

/// Most lines of an anchored block that are searched for. A finding that marks more is
/// anchored to the whole file. Not configurable: a bound on work per finding.
const MAX_BLOCK_LINES: usize = 2000;

/// What to locate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorQuery {
    pub sha: String,
    pub path: Option<String>,
    /// 1-based, inclusive.
    pub lines: Option<(u32, u32)>,
}

/// Where the code is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub status: AnchorStatus,
    pub now_path: Option<String>,
    /// For `untouched` lines that moved and for `lines edited`: where the lines are now.
    pub now_lines: Option<(u32, u32)>,
    /// True when the working tree was part of the check.
    pub worktree: bool,
}

/// The text of one file at one place.
enum Side {
    Text(String),
    /// Present but not text, or too large to read: only whole-file equality is possible.
    Opaque(Vec<u8>),
    Gone,
}

struct Reader<'a> {
    repo: &'a Repo,
    head: Option<String>,
    view: Option<WorktreeView>,
    memo: HashMap<(String, String), Side>,
    /// Renames from a commit to HEAD, read once per commit.
    renames: HashMap<String, HashMap<String, Option<String>>>,
}

impl Reader<'_> {
    fn blob(&mut self, rev: &str, path: &str) -> &Side {
        let key = (rev.to_owned(), path.to_owned());
        if !self.memo.contains_key(&key) {
            let limit = self.repo.text_limit();
            let side = match self.repo.raw_blob(rev, path, |_, size| size > limit) {
                Ok(b) => match b.bytes {
                    Some(bytes) => as_side(bytes),
                    None => Side::Opaque(b.id.into_bytes()),
                },
                Err(_) => Side::Gone,
            };
            self.memo.insert(key.clone(), side);
        }
        &self.memo[&key]
    }

    /// Where `path` of `sha` is at HEAD: itself, its new name after a rename, or `None`.
    fn at_head(&mut self, sha: &str, path: &str) -> Option<String> {
        let head = self.head.clone()?;
        if !matches!(self.blob(&head, path), Side::Gone) {
            return Some(path.to_owned());
        }
        if !self.renames.contains_key(sha) {
            let moved = self
                .repo
                .compare_files(sha, &head, false)
                .map(|(_, files)| {
                    files
                        .into_iter()
                        .filter_map(|f| f.old_path.map(|old| (old, Some(f.path))))
                        .collect()
                })
                .unwrap_or_default();
            self.renames.insert(sha.to_owned(), moved);
        }
        self.renames[sha].get(path).cloned().flatten()
    }
}

fn as_side(bytes: Vec<u8>) -> Side {
    if crate::diff::is_binary(&bytes) {
        return Side::Opaque(bytes);
    }
    match String::from_utf8(bytes) {
        Ok(t) => Side::Text(t),
        Err(e) => Side::Opaque(e.into_bytes()),
    }
}

fn same(a: &Side, b: &Side) -> bool {
    match (a, b) {
        (Side::Text(a), Side::Text(b)) => a == b,
        (Side::Opaque(a), Side::Opaque(b)) => a == b,
        (Side::Gone, Side::Gone) => true,
        _ => false,
    }
}

/// The 1-based inclusive range `start..=end` of `text`, `None` when it is not inside the text
/// or marks too much to search for.
fn block(text: &str, (start, end): (u32, u32)) -> Option<Vec<&str>> {
    let (s, e) = (start as usize, end as usize);
    if s == 0 || e < s || e - s >= MAX_BLOCK_LINES {
        return None;
    }
    let lines: Vec<&str> = text.lines().skip(s - 1).take(e - s + 1).collect();
    (lines.len() == e - s + 1).then_some(lines)
}

/// Where `want` sits in `text`: the start line (1-based) of the match nearest to `near`.
fn find_block(text: &str, want: &[&str], near: u32) -> Option<u32> {
    let all: Vec<&str> = text.lines().collect();
    if want.is_empty() || want.len() > all.len() {
        return None;
    }
    // Blank lines carry no identity: a block of them would match everywhere.
    let blank = want.iter().all(|l| l.trim().is_empty());
    let mut best: Option<u32> = None;
    for (i, w) in all.windows(want.len()).enumerate() {
        if w != want {
            continue;
        }
        let at = u32::try_from(i + 1).ok()?;
        if blank && at != near {
            continue;
        }
        if best.is_none_or(|b| at.abs_diff(near) < b.abs_diff(near)) {
            best = Some(at);
        }
    }
    best
}

/// What happened to the lines: found as they were, found elsewhere, or not found.
type Located = (AnchorStatus, Option<(u32, u32)>);

fn lines_status(old: &str, new: &str, range: (u32, u32)) -> Option<Located> {
    let want = block(old, range)?;
    let at = find_block(new, &want, range.0);
    Some(match at {
        Some(a) if a == range.0 => (AnchorStatus::Untouched, None),
        Some(a) => {
            let len = u32::try_from(want.len()).unwrap_or(u32::MAX);
            (AnchorStatus::Untouched, Some((a, a.saturating_add(len - 1))))
        }
        None => edited_span(old, new, range).map_or((AnchorStatus::LinesChanged, None), |span| {
            (AnchorStatus::LinesEdited, Some(span))
        }),
    })
}

/// One changed run of a line diff: old lines `.0` became new lines `.1`, each 0-based and
/// end-exclusive.
type Run = ((u32, u32), (u32, u32));

/// Where one old line is in the new text.
#[derive(Debug, PartialEq, Eq)]
enum Spot {
    /// In an unchanged run: the same line, at this 0-based place.
    Kept(u32),
    /// Inside a changed run: the lines that replaced it, 0-based and end-exclusive.
    Replaced(u32, u32),
}

fn spot(runs: &[Run], line: u32) -> Spot {
    let mut shift = 0i64;
    for &((old_start, old_end), (new_start, new_end)) in runs {
        if line < old_start {
            break;
        }
        if line < old_end {
            return Spot::Replaced(new_start, new_end);
        }
        shift += i64::from(new_end) - i64::from(new_start);
        shift -= i64::from(old_end) - i64::from(old_start);
    }
    Spot::Kept(u32::try_from(i64::from(line) + shift).unwrap_or(0))
}

/// The 1-based inclusive span in the new text of the old 0-based lines `first..=last`, or
/// `None` when every one of them was deleted. Never guesses: the ends come from the diff alone.
fn map_range(runs: &[Run], first: u32, last: u32) -> Option<(u32, u32)> {
    let start = match spot(runs, first) {
        Spot::Kept(n) | Spot::Replaced(n, _) => n,
    };
    let end = match spot(runs, last) {
        Spot::Kept(n) => n.saturating_add(1),
        Spot::Replaced(_, e) => e,
    };
    (end > start).then_some((start.saturating_add(1), end))
}

/// The lines of `old` at `range`, found in `new` by following the diff between them.
fn edited_span(old: &str, new: &str, range: (u32, u32)) -> Option<(u32, u32)> {
    let input = InternedInput::new(old, new);
    let diff = Diff::compute(Algorithm::Myers, &input);
    let runs: Vec<Run> = diff
        .hunks()
        .map(|h| ((h.before.start, h.before.end), (h.after.start, h.after.end)))
        .collect();
    map_range(&runs, range.0.checked_sub(1)?, range.1.checked_sub(1)?)
}

/// The anchors of `queries`, in the same order. The tree is listed and each blob read at most
/// once for all of them.
///
/// # Errors
/// Only when HEAD or the working tree cannot be listed at all.
pub fn anchors_now(repo: &Repo, queries: &[AnchorQuery]) -> Result<Vec<Anchor>> {
    let head = repo.resolve("HEAD").ok();
    let view = if repo.is_committed_only() { None } else { WorktreeView::open(repo).ok() };
    let mut r = Reader { repo, head, view, memo: HashMap::new(), renames: HashMap::new() };
    let live = r.view.as_ref().is_some_and(WorktreeView::is_live);
    let limit = repo.text_limit();
    let mut out = Vec::with_capacity(queries.len());
    for q in queries {
        out.push(anchor_of(&mut r, q, live, limit));
    }
    Ok(out)
}

fn anchor_of(r: &mut Reader<'_>, q: &AnchorQuery, live: bool, limit: u64) -> Anchor {
    let done = |status, now_path, now_lines| Anchor { status, now_path, now_lines, worktree: live };
    // A selection with no file (a commit, a page) is pinned to a commit, which never changes.
    let Some(path) = q.path.as_deref() else {
        return done(AnchorStatus::Untouched, None, None);
    };
    let Some(head) = r.head.clone() else {
        return done(AnchorStatus::FileGone, None, None);
    };
    let old = match r.blob(&q.sha, path) {
        Side::Text(t) => Side::Text(t.clone()),
        Side::Opaque(b) => Side::Opaque(b.clone()),
        Side::Gone => return done(AnchorStatus::FileGone, None, None),
    };
    let Some(now_path) = r.at_head(&q.sha, path) else {
        return gone_or_moved(r, path, &done);
    };
    let mut new = match r.blob(&head, &now_path) {
        Side::Text(t) => Side::Text(t.clone()),
        Side::Opaque(b) => Side::Opaque(b.clone()),
        Side::Gone => Side::Gone,
    };
    if let Some(view) = &r.view {
        match view.file(&now_path, limit) {
            WtFile::Same => {}
            WtFile::Gone => return gone_or_moved(r, &now_path, &done),
            WtFile::Data(d) => new = as_side(d),
            WtFile::Big => new = Side::Opaque(b"big".to_vec()),
        }
    }
    if same(&old, &new) {
        return done(AnchorStatus::Untouched, Some(now_path), None);
    }
    let located = match (&q.lines, &old, &new) {
        (Some(range), Side::Text(o), Side::Text(n)) => lines_status(o, n, *range),
        _ => None,
    };
    match located {
        Some((status, moved)) => done(status, Some(now_path), moved),
        None => done(AnchorStatus::FileChanged, Some(now_path), None),
    }
}

/// A file the working tree deleted may still be a rename of it: an exact move is `now_path`,
/// not `file gone`.
fn gone_or_moved(
    r: &Reader<'_>,
    path: &str,
    done: &dyn Fn(AnchorStatus, Option<String>, Option<(u32, u32)>) -> Anchor,
) -> Anchor {
    let moved = crate::worktree_diff(r.repo).ok().and_then(|files| {
        files.into_iter().find(|f| f.old_path.as_deref() == Some(path)).map(|f| f.path)
    });
    // Only an exact move is paired, so the content is the same under the new name.
    moved.map_or_else(
        || done(AnchorStatus::FileGone, None, None),
        |p| done(AnchorStatus::Untouched, Some(p), None),
    )
}

#[cfg(test)]
mod tests;
