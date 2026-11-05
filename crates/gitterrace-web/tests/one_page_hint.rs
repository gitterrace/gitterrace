//! A page's description sentence is one `.hint`: muted, small, the body font, spaced under the
//! tabs or the head. A `meta muted` paragraph that opens with prose is a hand-written copy: it
//! takes the mono metadata font and the tight metadata margin, so it reads as a sha line and
//! sits jammed under the navigation. `.meta` stays for real metadata (who, when, sha).

use std::path::Path;

/// `meta muted` paragraphs in `code` whose text opens with a capital letter followed by a
/// lower-case one, which marks a sentence and not metadata such as a template value or a name.
fn prose_metas(code: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for (i, _) in code.match_indices("<p class=\"meta muted\">") {
        let after = &code[i + "<p class=\"meta muted\">".len()..];
        let mut chars = after.chars();
        let first = chars.next().unwrap_or(' ');
        let second = chars.next().unwrap_or(' ');
        if first.is_ascii_uppercase() && second.is_ascii_lowercase() {
            bad.push(code[i..].chars().take(80).collect());
        }
    }
    bad
}

#[test]
fn a_prose_meta_line_is_caught() {
    assert_eq!(
        prose_metas("<p class=\"meta muted\">Linked worktrees recorded here.</p>").len(),
        1
    );
    assert!(prose_metas("<p class=\"meta muted\">{% if a %}Tagged{% endif %}</p>").is_empty());
    assert!(prose_metas("<p class=\"hint\">Linked worktrees recorded here.</p>").is_empty());
    assert!(prose_metas("<p class=\"meta\"><b>Ann</b> 2 days ago</p>").is_empty());
}

#[test]
fn page_descriptions_use_the_hint() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut bad = Vec::new();
    for e in std::fs::read_dir(&dir).expect("templates").flatten() {
        let p = e.path();
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        for b in prose_metas(&text) {
            bad.push(format!("{}: {b}", p.display()));
        }
    }
    assert!(
        bad.is_empty(),
        "description written as metadata, use class=\"hint\":\n{}",
        bad.join("\n")
    );
}
