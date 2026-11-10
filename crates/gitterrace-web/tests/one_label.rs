//! Every small label (a `badge` or a `tag`) comes from the macros in `_badge.html`, so its
//! class, modifier and escaping stay one markup. A template that writes either class by hand
//! drifts from it.

use std::path::Path;

use gitterrace_testkit::scan;

/// Files that may write the classes: the macros, and the diff file header, which writes its
/// change-kind badge in the per-file loop of `views/diff.rs` with the rest of that header.
const OWNERS: [&str; 2] = ["_badge.html", "diff.rs"];

/// Hand-written labels in `code`: an element whose class list starts with `badge` or `tag`.
fn loose_labels(code: &str) -> usize {
    code.match_indices("class=")
        .filter(|(i, _)| {
            let rest = code[i + 6..].trim_start_matches(['"', '\\']);
            let first = rest.split(['"', '\\', ' ']).next().unwrap_or_default();
            first == "badge" || first == "tag" || first == "badge{%" || first == "tag{%"
        })
        .count()
}

#[test]
fn a_hand_written_label_is_caught() {
    assert_eq!(loose_labels(r#"<span class="tag">new</span>"#), 1);
    assert_eq!(loose_labels(r#"<span class="badge set-src">x</span>"#), 1);
    assert_eq!(
        loose_labels(r#"<span class="tag {{ b }}">{{ b }}</span>"#),
        1
    );
    assert_eq!(
        loose_labels(r#"format!("<span class=\"badge\">x</span>")"#),
        1
    );
    assert_eq!(
        loose_labels(r#"<a class="tags">x</a><b class="sha">y</b>"#),
        0
    );
}

#[test]
fn only_the_label_macros_write_a_badge_or_tag() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    for p in scan::files_in(root, &["templates", "src"], &[]) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
        if name
            .as_deref()
            .is_some_and(|n| OWNERS.contains(&n) || n.ends_with("tests.rs"))
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        if loose_labels(scan::product(&text)) > 0 {
            bad.push(p.display().to_string());
        }
    }
    assert!(bad.is_empty(), "hand-written badge or tag in {bad:?}");
}
