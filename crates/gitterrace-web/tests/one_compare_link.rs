//! Compare sits under Commits, and every link to it comes from `compare::compare_href`, which
//! applies the feature switch and the prefill. A link built by hand forgets one of them, which
//! is how a branch row once linked to a compare page the config had turned off.

use std::path::Path;

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
    let mut stack = vec![
        root.join("src"),
        root.join("templates"),
        root.join("static"),
    ];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
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
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            for b in hand_links(code) {
                bad.push(format!("{}: {b}", p.display()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "hand-built compare link:\n{}",
        bad.join("\n")
    );
}
