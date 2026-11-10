//! Compare sits under Commits, and every link to it comes from `compare::compare_href`, which
//! applies the feature switch and the prefill. A link built by hand forgets one of them, which
//! is how a branch row once linked to a compare page the config had turned off.

use std::path::Path;

use gitterrace_testkit::scan;

/// Spots of `code` that build a compare URL by hand.
fn hand_links(code: &str) -> Vec<String> {
    code.match_indices("/compare?")
        .map(|(i, _)| code[i.saturating_sub(30)..i + 9].to_owned())
        .collect()
}

#[test]
fn a_hand_built_compare_link_is_caught() {
    let row = "href(&format!(\"/compare?base={}&head={}\", a, b))";
    assert_eq!(hand_links(row).len(), 1);
    assert!(hand_links("crate::compare::compare_href(Some(b), h)").is_empty());
}

/// Only `compare.rs` builds the URL; templates link with what it returns.
#[test]
fn only_the_compare_helper_builds_its_link() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    for p in scan::files_in(root, &["src", "templates", "static"], &[]) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
        // The owner, and test files that read links back.
        if name
            .as_deref()
            .is_some_and(|n| n == "compare.rs" || n.ends_with("tests.rs"))
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        for b in hand_links(scan::product(&text)) {
            bad.push(format!("{}: {b}", p.display()));
        }
    }
    assert!(
        bad.is_empty(),
        "hand-built compare link:\n{}",
        bad.join("\n")
    );
}
