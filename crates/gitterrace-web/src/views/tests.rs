use crate::views::{code_html, code_html_first, code_tail_html, crumbs, diff_html};

const CACHE: usize = 32 << 20;
use gitterrace_git::{ChangeKind, FileDiff, LineKind};

#[test]
fn crumbs_link_prefixes() {
    let c = crumbs("main", "a/b.rs", true);
    assert_eq!(c.len(), 3);
    assert_eq!(c[1].href, "/tree/main/a");
    assert_eq!(c[2].href, "/blob/main/a/b.rs");
}

#[test]
fn tail_of_a_file_without_late_chunks_is_empty() {
    let ten = "fn main() {}\n".repeat(10);
    let two_chunks = "fn main() {}\n".repeat(150);
    for (path, text) in [
        ("a.rs", ""),
        ("a.rs", ten.as_str()),
        ("a.rs", two_chunks.as_str()),
        ("a.unknownext", ten.as_str()),
    ] {
        assert_eq!(code_tail_html(CACHE, path, text), "", "{path}");
    }
}

#[test]
fn code_escapes() {
    let (h, cut) = code_html(CACHE, "x.txt", "<b>\nx");
    assert!(h.contains("&lt;b&gt;"));
    assert!(!cut);
    assert!(h.contains("id=\"L2\""));
    assert!(!h.contains("<table"));
}

#[test]
fn big_files_light_the_first_chunks_and_the_tail_completes_them() {
    let text = "fn f() { let x = 1; }\n".repeat(3_000);
    let (first, _) = code_html_first(CACHE, "a.rs", &text);
    assert!(first.starts_with("<div class=\"code src\" data-hl>"));
    let lit = |h: &str| h.matches("s-keyword").count();
    assert!(lit(&first) > 0 && lit(&first) < 3_000 * 2);
    let (full, _) = code_html(CACHE, "a.rs", &text);
    let tail = code_tail_html(CACHE, "a.rs", &text);
    let head_end = first.match_indices("<div class=\"ch\"").nth(2).unwrap().0;
    let glued = format!(
        "{}{}</div>",
        &full[..full.match_indices("<div class=\"ch\"").nth(2).unwrap().0],
        tail
    );
    assert_eq!(glued.replace(" data-hl", ""), full);
    assert!(first[..head_end].contains("s-keyword"));
    let (small, _) = code_html_first(CACHE, "a.rs", "fn f() {}\n");
    assert!(!small.contains("data-hl"));
}

#[tokio::test]
async fn diff_fragment_out_of_range_is_not_found() {
    use axum::extract::{Path, State};
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let _fixture = gitterrace_testkit::Fixture::init(p)
        .write(
            "a.txt", "x
",
        )
        .commit("feat: a");
    let repo = gitterrace_git::Repo::discover(p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = std::sync::Arc::new(crate::app::App::new(repo, index));
    let ask = |i: usize| super::diff_fragment(State(app.clone()), Path(("HEAD".into(), i)));
    assert!(ask(0).await.is_ok());
    assert!(ask(99_999).await.is_err());
}

#[test]
fn code_lines_carry_their_number() {
    let text = "l\n".repeat(250);
    let (h, _) = code_html(CACHE, "x.txt", &text);
    assert_eq!(h.matches("class=\"ch\"").count(), 3);
    // A strict CSP refuses inline styles, so each line names its own number.
    assert!(h.contains("id=\"L101\" class=\"l\" data-n=\"101\""));
    assert!(h.contains("id=\"L250\""));
    assert!(!h.contains("style="));
}

#[test]
fn huge_diffs_fold_behind_a_load_button() {
    use gitterrace_git::{CodeCounts, Hunk, HunkLine};
    let lines = |n: usize| {
        (0..n)
            .map(|i| HunkLine {
                kind: LineKind::Added,
                old_no: None,
                new_no: Some(i as u32 + 1),
                text: "x".into(),
            })
            .collect::<Vec<_>>()
    };
    let file = |n: usize| FileDiff {
        path: "a.rs".into(),
        old_path: None,
        kind: ChangeKind::Modified,
        binary: false,
        additions: n as u32,
        deletions: 0,
        hunks: vec![Hunk {
            header: "@@".into(),
            lines: lines(n),
        }],
        lines: n as u32,
        rewritten: 0,
        code: CodeCounts::default(),
        modes: None,
    };
    let small = diff_html(400, "abc", None, &[file(10)]);
    assert!(!small.contains("data-load-diff"));
    let big = diff_html(400, "abc", None, &[file(401)]);
    assert!(!diff_html(500, "abc", None, &[file(401)]).contains("data-load-diff"));
    assert!(big.contains("data-load-diff=\"/diff/abc/0\""));
    assert!(!big.contains("<div class=\"add\""));
}
