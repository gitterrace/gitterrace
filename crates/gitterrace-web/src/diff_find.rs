//! Find in a whole diff: the server reads every file of a commit or of the working tree, so a
//! folded file and one not yet loaded are searched too. The page lists the hits and opens a file
//! on a click.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use gitterrace_git::{FileDiff, LineKind};
use serde::{Deserialize, Serialize};

use crate::{app::blocking, error::Result, routes::Shared, search::MAX_QUERY_BYTES};

/// Hit lines listed per file, and hits listed in all: the counts stay exact, the lines stop.
/// Not configurable: the list is a preview, the counts above it are complete.
const LINES_PER_FILE: usize = 3;
const MAX_HITS: usize = 500;
const LINE_CHARS: usize = 160;

#[derive(Deserialize, Default)]
pub(crate) struct FindQuery {
    q: Option<String>,
    scope: Option<String>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
struct Line {
    no: Option<u32>,
    text: String,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
struct Hit {
    path: String,
    count: usize,
    lines: Vec<Line>,
}

#[derive(Serialize, Debug, PartialEq, Eq, Default)]
pub(crate) struct Found {
    files: Vec<Hit>,
    /// Hits over every file, listed or not.
    total: usize,
    /// More files hit than are listed.
    truncated: bool,
}

/// The files of `files` whose changed or context lines hold `needle` (any case).
fn find(files: &[FileDiff], needle: &str) -> Found {
    let mut out = Found::default();
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return out;
    }
    for f in files {
        let mut hit = Hit { path: f.path.clone(), count: 0, lines: Vec::new() };
        for l in f.hunks.iter().flat_map(|h| &h.lines) {
            if !l.text.to_lowercase().contains(&needle) {
                continue;
            }
            hit.count += 1;
            if hit.lines.len() < LINES_PER_FILE {
                hit.lines.push(Line {
                    no: if l.kind == LineKind::Removed { l.old_no } else { l.new_no },
                    text: l.text.chars().take(LINE_CHARS).collect(),
                });
            }
        }
        if hit.count == 0 {
            continue;
        }
        out.total += hit.count;
        if out.files.len() < MAX_HITS {
            out.files.push(hit);
        } else {
            out.truncated = true;
        }
    }
    out
}

fn needle(q: &FindQuery) -> String {
    let q = q.q.as_deref().unwrap_or("");
    let mut end = q.len().min(MAX_QUERY_BYTES);
    while !q.is_char_boundary(end) {
        end -= 1;
    }
    q[..end].to_owned()
}

/// `GET /diff-find/{rev}?q=`: the files of a commit's diff holding `q`.
pub(crate) async fn commit(
    State(app): Shared,
    Path(rev): Path<String>,
    Query(q): Query<FindQuery>,
) -> Result<Json<Found>> {
    let n = needle(&q);
    let found = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        Ok(find(&a.repo.commit(&id)?.files, &n))
    })
    .await?;
    Ok(Json(found))
}

/// `GET /worktree/find?q=&scope=`: the same over the working tree tab.
pub(crate) async fn worktree(
    State(app): Shared,
    Query(q): Query<FindQuery>,
) -> Result<Json<Found>> {
    let n = needle(&q);
    let scope = crate::worktree_scope::named(q.scope.as_deref());
    let found = blocking(&app, move |a| {
        Ok(find(&gitterrace_git::worktree_diff_scoped(&a.repo, scope)?, &n))
    })
    .await?;
    Ok(Json(found))
}

#[cfg(test)]
mod tests {
    use gitterrace_git::{ChangeKind, CodeCounts, Hunk, HunkLine};

    use super::*;

    fn file(path: &str, texts: &[&str]) -> FileDiff {
        FileDiff {
            path: path.into(),
            kind: ChangeKind::Modified,
            old_path: None,
            additions: 0,
            deletions: 0,
            binary: false,
            hunks: vec![Hunk {
                header: "@@".into(),
                lines: texts
                    .iter()
                    .enumerate()
                    .map(|(i, t)| HunkLine {
                        kind: LineKind::Added,
                        old_no: None,
                        new_no: Some(i as u32 + 1),
                        text: (*t).into(),
                    })
                    .collect(),
            }],
            lines: 0,
            rewritten: 0,
            code: CodeCounts::default(),
            modes: None,
        }
    }

    #[test]
    fn a_query_matches_any_case_and_counts_every_line() {
        let files = [file("a.rs", &["Foo", "bar", "fOO again"]), file("b.rs", &["x"])];
        let r = find(&files, "foo");
        assert_eq!((r.files.len(), r.total), (1, 2));
        assert_eq!(r.files[0].lines[1].no, Some(3));
    }

    #[test]
    fn an_empty_query_finds_nothing() {
        assert_eq!(find(&[file("a.rs", &["x"])], ""), Found::default());
    }

    #[test]
    fn a_file_lists_a_few_lines_but_counts_all() {
        let t = ["hit"; 5];
        let r = find(&[file("a.rs", &t)], "hit");
        assert_eq!((r.files[0].count, r.files[0].lines.len()), (5, LINES_PER_FILE));
    }
}
