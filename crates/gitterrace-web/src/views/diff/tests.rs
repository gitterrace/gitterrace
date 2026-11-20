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
    let html =
        diff_html(&crate::views::Render::fold(400), "abc", None, &[moved("old/a.rs", "new/a.rs")]);
    let s = section(&html, 0);
    assert!(
        s.contains(
            "<p class=\"muted pad moved\">Renamed from <code>old/a.rs</code>, no content \
            change.</p>"
        ),
        "{s}"
    );
    assert!(!s.contains("class=\"diff\"") && !s.contains("No textual changes"), "{s}");
}

#[test]
fn a_mode_only_change_is_one_line() {
    let f = FileDiff {
        hunks: Vec::new(),
        additions: 0,
        modes: Some((0o100_644, 0o100_755)),
        ..file("run.sh", 0)
    };
    let s = diff_html(&crate::views::Render::fold(400), "abc", None, &[f]);
    assert!(s.contains("Mode changed from 100644 to 100755, no content change."), "{s}");
    let both = FileDiff { modes: Some((0o100_644, 0o100_755)), ..moved("a.sh", "b.sh") };
    let s = diff_html(&crate::views::Render::fold(400), "abc", None, &[both]);
    assert!(
        s.contains("Renamed from <code>a.sh</code>, mode changed from 100644 to 100755"),
        "{s}"
    );
}

#[test]
fn files_past_the_page_budget_fold_lazily_and_link_their_body() {
    let files: Vec<FileDiff> = (0..120).map(|i| file(&format!("f{i}.rs"), 3)).collect();
    let html = diff_html(&crate::views::Render::fold(400), "abc", None, &files);
    let open = html.matches("<div class=\"diffwrap\">").count();
    assert_eq!(open, super::TOTAL_DIFF_FILES, "the first files render in full");
    let last = section(&html, 119);
    assert!(
        last.contains("href=\"/diff/abc/119\" data-load-diff=\"/diff/abc/119\" data-lazy"),
        "{last}"
    );
    // A file over its own limit folds without data-lazy: it waits for a click.
    let big = diff_html(&crate::views::Render::fold(400), "abc", None, &[file("big.rs", 401)]);
    assert!(big.contains("data-load-diff") && !big.contains("data-lazy"), "{big}");
    assert!(big.contains("data-load-diff=\"/diff/abc/0\"") && !big.contains("<div class=\"add\""));
    let at_limit = diff_html(&crate::views::Render::fold(401), "abc", None, &[file("big.rs", 401)]);
    assert!(!at_limit.contains("data-load-diff"), "a file within the limit stays open");
}

#[test]
fn renames_spend_no_budget() {
    let mut files: Vec<FileDiff> =
        (0..1_000).map(|i| moved(&format!("a/{i}"), &format!("b/{i}"))).collect();
    files.push(file("late.rs", 3));
    let html = diff_html(&crate::views::Render::fold(400), "abc", None, &files);
    assert!(section(&html, 1_000).contains("class=\"diffwrap\""));
}

#[test]
fn the_working_tree_folds_too_and_loads_by_path() {
    let files: Vec<FileDiff> = (0..60).map(|i| file(&format!("d/f {i}.rs"), 3)).collect();
    let html = diff_html_with(
        &crate::views::Render::fold(400),
        DiffSource::Worktree {
            generated: &std::collections::HashSet::new(),
            scope: gitterrace_git::Scope::All,
        },
        &files,
    );
    assert!(section(&html, 59).contains("data-load-diff=\"/worktree/diff/d/f%2059.rs\""), "{html}");
}

/// Each added and removed line says so in words before its text, plain and highlighted, and a
/// context line says nothing. `gitterrace_testkit::a11y` checks the same on every page.
#[test]
fn every_changed_line_says_added_or_removed() {
    let line = |kind, old_no, new_no| HunkLine { kind, old_no, new_no, text: "let x = 1;".into() };
    let f = FileDiff {
        additions: 1,
        deletions: 1,
        hunks: vec![Hunk {
            header: "@@".into(),
            lines: vec![
                line(LineKind::Context, Some(1), Some(1)),
                line(LineKind::Removed, Some(2), None),
                line(LineKind::Added, None, Some(2)),
            ],
        }],
        ..file("a.rs", 0)
    };
    for plain in [false, true] {
        let mut r = crate::views::Render::fold(400);
        if plain {
            r.highlight_bytes = 0;
        }
        let html = diff_html(&r, "abc", None, std::slice::from_ref(&f));
        let problems = gitterrace_testkit::a11y::problems(&format!("<h1>x</h1>{html}"));
        assert!(problems.is_empty(), "{problems:?} in {html}");
        assert!(html.contains("\"><span class=\"vh sign\">Removed, </span>"), "{html}");
        assert!(html.contains("\"><span class=\"vh sign\">Added, </span>"), "{html}");
        assert_eq!(html.matches("vh sign").count(), 2, "{html}");
    }
}

#[test]
fn generated_files_are_counted_above_the_diff_with_a_way_to_expand_them() {
    let files = vec![file("a.rs", 3), file("Cargo.lock", 3), file("b.lock", 3)];
    let generated: std::collections::HashSet<String> =
        ["Cargo.lock".to_owned(), "b.lock".to_owned()].into();
    let none = super::MediaFacts::new();
    let src = |g| DiffSource::Commit { rev: "abc", parent: None, media: &none, generated: g };
    let html = diff_html_with(&crate::views::Render::fold(400), src(&generated), &files);
    assert!(html.contains("2 generated files folded"), "{html}");
    assert!(html.contains("data-expand-generated"));
    let empty = std::collections::HashSet::new();
    let html = diff_html_with(&crate::views::Render::fold(400), src(&empty), &files);
    assert!(!html.contains("folded"), "{html}");
}
