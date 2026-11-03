//! Unified hunks with context lines from a line diff.

use std::ops::Range;

use gix::bstr::ByteSlice;
use gix::diff::blob::Diff;

use crate::{Hunk, HunkLine, LineKind};

const CONTEXT: usize = 3;

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
