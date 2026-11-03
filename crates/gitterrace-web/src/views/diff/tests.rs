use gitterrace_git::{ChangeKind, CodeCounts, FileDiff, Hunk, HunkLine, LineKind};

use super::{DiffSource, diff_html, diff_html_with};

fn file(path: &str, changed: usize) -> FileDiff {
    FileDiff {
        path: path.into(),
        old_path: None,
        kind: ChangeKind::Modified,
        binary: false,
        additions: changed as u32,
        deletions: 0,
        hunks: vec![Hunk {
            header: "@@".into(),
            lines: (0..changed)
                .map(|i| HunkLine {
                    kind: LineKind::Added,
                    old_no: None,
                    new_no: Some(i as u32 + 1),
                    text: "x".into(),
                })
                .collect(),
        }],
        lines: changed as u32,
        rewritten: 0,
        code: CodeCounts::default(),
        modes: None,
    }
}

fn moved(from: &str, to: &str) -> FileDiff {
    FileDiff {
        old_path: Some(from.into()),
        kind: ChangeKind::Renamed,
        additions: 0,
        hunks: Vec::new(),
        ..file(to, 0)
    }
}

/// The section of file `i`, from its opening tag to the next one.
fn section(html: &str, i: usize) -> &str {
    let at = html.find(&format!("id=\"f{i}\"")).expect("section");
    let rest = &html[at..];
    &rest[..rest.find("</section>").expect("end")]
}

#[test]
fn a_pure_rename_is_one_line_not_a_diff() {
    let html = diff_html(400, "abc", None, &[moved("old/a.rs", "new/a.rs")]);
    let s = section(&html, 0);
    assert!(
        s.contains("<p class=\"muted pad moved\">Renamed from <code>old/a.rs</code>, no content change.</p>"),
        "{s}"
    );
    assert!(
        !s.contains("class=\"diff\"") && !s.contains("No textual changes"),
        "{s}"
    );
}

#[test]
fn a_mode_only_change_is_one_line() {
    let f = FileDiff {
        hunks: Vec::new(),
        additions: 0,
        modes: Some((0o100_644, 0o100_755)),
        ..file("run.sh", 0)
    };
    let s = diff_html(400, "abc", None, &[f]);
    assert!(
        s.contains("Mode changed from 100644 to 100755, no content change."),
        "{s}"
    );
    let both = FileDiff {
        modes: Some((0o100_644, 0o100_755)),
        ..moved("a.sh", "b.sh")
    };
    let s = diff_html(400, "abc", None, &[both]);
    assert!(
        s.contains("Renamed from <code>a.sh</code>, mode changed from 100644 to 100755"),
        "{s}"
    );
}

#[test]
fn files_past_the_page_budget_fold_lazily_and_link_their_body() {
    let files: Vec<FileDiff> = (0..120).map(|i| file(&format!("f{i}.rs"), 3)).collect();
    let html = diff_html(400, "abc", None, &files);
    let open = html.matches("<div class=\"diffwrap\">").count();
    assert_eq!(
        open,
        super::TOTAL_DIFF_FILES,
        "the first files render in full"
    );
    let last = section(&html, 119);
    assert!(
        last.contains("href=\"/diff/abc/119\" data-load-diff=\"/diff/abc/119\" data-lazy"),
        "{last}"
    );
    // A file over its own limit folds without data-lazy: it waits for a click.
    let big = diff_html(400, "abc", None, &[file("big.rs", 401)]);
    assert!(
        big.contains("data-load-diff") && !big.contains("data-lazy"),
        "{big}"
    );
}

#[test]
fn renames_spend_no_budget() {
    let mut files: Vec<FileDiff> = (0..1_000)
        .map(|i| moved(&format!("a/{i}"), &format!("b/{i}")))
        .collect();
    files.push(file("late.rs", 3));
    let html = diff_html(400, "abc", None, &files);
    assert!(section(&html, 1_000).contains("class=\"diffwrap\""));
}

#[test]
fn the_working_tree_folds_too_and_loads_by_path() {
    let files: Vec<FileDiff> = (0..60).map(|i| file(&format!("d/f {i}.rs"), 3)).collect();
    let html = diff_html_with(400, DiffSource::Worktree, &files);
    assert!(
        section(&html, 59).contains("data-load-diff=\"/worktree/diff/d/f%2059.rs\""),
        "{html}"
    );
}
