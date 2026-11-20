use std::fmt::Write as _;

use super::*;

fn attrs(docs: &[&str]) -> String {
    let mut s = String::from("* text=auto\n# a comment: linguist-generated\n");
    for d in docs.iter().chain(&BENCH) {
        let _ = writeln!(s, "{d} linguist-generated");
    }
    s
}

fn outs(paths: &[&str]) -> Vec<Out> {
    paths.iter().map(|p| Out::file(*p, "x")).collect()
}

#[test]
fn a_marked_generated_doc_passes() {
    let a = attrs(&["docs/api.md", "docs/openapi.json"]);
    assert!(problems(&a, &outs(&["docs/api.md", "docs/openapi.json"])).is_empty());
}

/// The class this guards: a generator's document added without its mark, of any extension.
#[test]
fn a_generated_doc_without_a_mark_is_reported() {
    let a = attrs(&["docs/api.md"]);
    let bad = problems(
        &a,
        &outs(&["docs/api.md", "docs/other.md", "docs/new.json"]),
    );
    assert_eq!(bad.len(), 2, "{bad:?}");
    assert!(bad[0].starts_with("docs/other.md") && bad[1].starts_with("docs/new.json"));
}

/// The class this guards: a mark that outlived its generator, so a hand-written page folds.
#[test]
fn a_mark_on_a_doc_nothing_generates_is_reported() {
    let a = attrs(&["docs/api.md", "docs/gone.md"]);
    let bad = problems(&a, &outs(&["docs/api.md"]));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].contains("docs/gone.md"));
}

#[test]
fn a_dropped_bench_pattern_is_reported() {
    let a = attrs(&[]).replace("tests/bench/results/**/*.svg linguist-generated\n", "");
    let bad = problems(&a, &[]);
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].contains("**/*.svg"));
}

#[test]
fn a_comment_is_not_a_mark() {
    assert_eq!(
        marked("# docs/x.md linguist-generated\n"),
        Vec::<&str>::new()
    );
}

/// The drift check: the tree's own `.gitattributes` marks what the generators write.
#[test]
fn the_tree_marks_every_generated_doc() {
    let root = crate::repo_root();
    let outs = crate::generators(&root, false).expect("generators");
    let bad = check(&root, &outs).expect("check");
    assert!(bad.is_empty(), "{bad:?}");
}
