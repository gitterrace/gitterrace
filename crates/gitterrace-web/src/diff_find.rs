//! Find in a whole diff: the server reads every file of a commit or of the working tree, so a
//! folded file and one not yet loaded are searched too. The page lists the hits and opens a file
//! on a click.

use askama::Template;
use axum::{
    Json,
    extract::{Path, Query, State},
};
use gitterrace_git::{FileDiff, LineKind};
use serde::{Deserialize, Serialize};

use crate::{app::blocking, error::Result, routes::Shared, search::MAX_QUERY_BYTES, views::Folded};

/// Lines listed in all with their match ranges: the counts stay exact, the list stops. Not
/// configurable: the page steps through a preview of the hits, the counts are complete.
const MAX_LINES: usize = 2000;

#[derive(Deserialize, Default)]
pub(crate) struct FindQuery {
    q: Option<String>,
    scope: Option<String>,
    since: Option<String>,
}

/// One matching line: where it sits in its file (`kind` and `no` name the line the page marks)
/// and the UTF-16 ranges of the needle in its text, as the browser counts characters.
#[derive(Serialize, Debug, PartialEq, Eq)]
struct Match {
    kind: &'static str,
    no: Option<u32>,
    ranges: Vec<[u32; 2]>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
struct Hit {
    path: String,
    count: usize,
    matches: Vec<Match>,
}

#[derive(Serialize, Debug, PartialEq, Eq, Default)]
pub(crate) struct Found {
    files: Vec<Hit>,
    /// Hits over every file, listed or not.
    total: usize,
    /// More lines hit than are listed.
    truncated: bool,
}

/// The case folding the search uses: one char to one char, so offsets in the folded text are
/// offsets in the original. A char that lowercases to several stays as it is.
fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

/// The ranges of `needle` (already folded) in `text`, non-overlapping, in UTF-16 units.
fn ranges(text: &str, needle: &[char]) -> Vec<[u32; 2]> {
    let chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = chars.iter().map(|c| fold(*c)).collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i + needle.len() <= folded.len() {
        if folded[i..i + needle.len()] == *needle {
            found.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    if found.is_empty() {
        return Vec::new();
    }
    let mut at = Vec::with_capacity(chars.len() + 1);
    let mut n = 0u32;
    for c in &chars {
        at.push(n);
        n += u32::try_from(c.len_utf16()).unwrap_or(1);
    }
    at.push(n);
    found.into_iter().map(|(s, e)| [at[s], at[e]]).collect()
}

/// The files of `files` whose changed or context lines hold `needle` (any case).
fn find(files: &[FileDiff], needle: &str) -> Found {
    let mut out = Found::default();
    let needle: Vec<char> = needle.chars().map(fold).collect();
    if needle.is_empty() {
        return out;
    }
    let mut listed = 0;
    for f in files {
        let mut hit = Hit { path: f.path.clone(), count: 0, matches: Vec::new() };
        for l in f.hunks.iter().flat_map(|h| &h.lines) {
            let r = ranges(&l.text, &needle);
            if r.is_empty() {
                continue;
            }
            hit.count += 1;
            if listed >= MAX_LINES {
                out.truncated = true;
                continue;
            }
            listed += 1;
            let (kind, no) = match l.kind {
                LineKind::Removed => ("del", l.old_no),
                LineKind::Added => ("add", l.new_no),
                LineKind::Context => ("ctx", l.new_no),
            };
            hit.matches.push(Match { kind, no, ranges: r });
        }
        if hit.count > 0 {
            out.total += hit.count;
            out.files.push(hit);
        }
    }
    out
}

/// The server-rendered bar on the changed-files card: viewed progress, the find field, its
/// status and steppers, the key hint and the hide-viewed switch. The scripts only fill it in.
#[derive(Template)]
#[template(path = "difftools.html")]
struct Toolbar {
    files: usize,
    label: String,
    /// How many files are open and how many wait behind a link, empty when all are open.
    open: String,
    has_folded: bool,
}

/// The bar for a diff of `files` files, `folded` of them behind a Load diff link. Empty when
/// there is nothing to find in.
pub(crate) fn toolbar(files: usize, folded: Folded) -> String {
    if files == 0 {
        return String::new();
    }
    let label = format!("Viewed 0 of {files}");
    let has_folded = folded.files > 0;
    let open = if has_folded {
        format!(
            "{} of {} open, {} folded ({})",
            files.saturating_sub(folded.files),
            crate::fmt::count(files, "file", "files"),
            folded.files,
            crate::fmt::count(folded.lines, "line", "lines")
        )
    } else {
        String::new()
    };
    Toolbar { files, label, open, has_folded }.render().unwrap_or_default()
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
    let since = q.since.filter(|s| !s.is_empty());
    let found = blocking(&app, move |a| {
        let files = match &since {
            Some(id) => crate::checkpoints_web::since_files(a, id)?,
            None => gitterrace_git::worktree_diff_scoped(&a.repo, scope)?,
        };
        Ok(find(&files, &n))
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
        assert_eq!(r.files[0].matches[1].no, Some(3));
        assert_eq!(r.files[0].matches[1].ranges, [[0, 3]]);
    }

    #[test]
    fn an_empty_query_finds_nothing() {
        assert_eq!(find(&[file("a.rs", &["x"])], ""), Found::default());
    }

    #[test]
    fn a_file_counts_every_line_and_the_list_stops_at_its_cap() {
        let t = vec!["hit"; MAX_LINES + 3];
        let r = find(&[file("a.rs", &t)], "hit");
        assert_eq!((r.files[0].count, r.files[0].matches.len()), (MAX_LINES + 3, MAX_LINES));
        assert!(r.truncated);
    }

    #[test]
    fn ranges_count_utf16_units_and_fold_case_across_scripts() {
        // The emoji is two UTF-16 units, so the match after it starts at 3, not 2.
        let n: Vec<char> = "ünï".chars().map(fold).collect();
        assert_eq!(ranges("\u{1F600}x ÜNÏ ünï", &n), [[4, 7], [8, 11]]);
        // One char to one char: a char that lowercases to two is left as it is.
        let n: Vec<char> = "i".chars().map(fold).collect();
        assert_eq!(ranges("\u{130}i", &n), [[1, 2]]);
    }

    #[test]
    fn ranges_never_overlap() {
        let n: Vec<char> = "aa".chars().collect();
        assert_eq!(ranges("aaaaa", &n), [[0, 2], [2, 4]]);
    }

    #[test]
    fn a_removed_line_is_named_by_its_old_number() {
        let mut f = file("a.rs", &["gone"]);
        f.hunks[0].lines[0].kind = LineKind::Removed;
        f.hunks[0].lines[0].old_no = Some(9);
        assert_eq!(find(&[f], "gone").files[0].matches[0].kind, "del");
    }

    #[test]
    fn the_bar_is_empty_without_files_and_names_the_total() {
        assert_eq!(toolbar(0, Folded::default()), "");
        let all_open = toolbar(9, Folded::default());
        assert!(all_open.contains("Viewed 0 of 9"));
        assert!(!all_open.contains("data-open-all"), "{all_open}");
    }

    #[test]
    fn the_bar_counts_folded_files_and_offers_open_all() {
        let bar = toolbar(41, Folded { files: 3, lines: 9800 });
        assert!(bar.contains("38 of 41 files open, 3 folded (9,800 lines)"), "{bar}");
        assert!(bar.contains("data-open-all"), "{bar}");
    }
}
