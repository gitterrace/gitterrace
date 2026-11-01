//! AGENTS.md rules that a grep can hold: they fail here instead of in review.

use std::{fs, path::Path};

/// Every image in a template needs alt text, empty for decoration.
#[test]
fn template_images_have_alt() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    for entry in fs::read_dir(dir).expect("templates dir").flatten() {
        let Ok(html) = fs::read_to_string(entry.path()) else {
            continue;
        };
        for tag in html.split("<img").skip(1) {
            let tag = tag.split('>').next().unwrap_or_default();
            assert!(
                tag.contains("alt="),
                "{} has an <img> without alt",
                entry.path().display()
            );
        }
    }
}

/// Every copy goes through `gtCopy` in app.js, which flashes the button and shows the toast. A
/// script that writes the clipboard itself copies silently, and the user cannot tell it worked.
#[test]
fn clipboard_writes_go_through_gt_copy() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("static dir").flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "js") || path.ends_with("app.js") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read script");
        checked += 1;
        for needle in [
            "clipboard.writeText",
            "execCommand(\"copy\")",
            "execCommand('copy')",
        ] {
            assert!(
                !text.contains(needle),
                "{} calls {needle}: copy through window.gtCopy so the copy shows an indicator",
                path.display()
            );
        }
    }
    assert!(checked > 0, "no scripts found in {}", dir.display());
}

/// Nouns that a template must never put straight after a number: `{{ n }} commits` says
/// "1 commits". Render the pair from Rust with `fmt::count` or `fmt::noun`.
const COUNTED_NOUNS: [&str; 14] = [
    "commits",
    "authors",
    "files",
    "lines",
    "days",
    "weeks",
    "repositories",
    "contributors",
    "changes",
    "matches",
    "branches",
    "tags",
    "people",
    "hunks",
];

/// `}}` or `}}</b>`, whitespace, then a counted noun, as a word.
fn fixed_plural(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(at) = rest.find("}}") {
        rest = &rest[at + 2..];
        let tail = rest.strip_prefix("</b>").unwrap_or(rest);
        let word = tail.trim_start_matches([' ', '\t', '\n']);
        if word.len() == tail.len() {
            continue;
        }
        let end = word
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(word.len());
        if COUNTED_NOUNS.contains(&&word[..end]) {
            return Some(format!("}}}} {}", &word[..end]));
        }
    }
    None
}

#[test]
fn templates_never_pair_a_number_with_a_fixed_plural() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    for entry in fs::read_dir(dir).expect("templates dir").flatten() {
        let Ok(html) = fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(hit) = fixed_plural(&html) {
            panic!(
                "{} renders `{hit}`: pass the counted text from Rust (fmt::count / fmt::noun)",
                entry.path().display()
            );
        }
    }
}

#[test]
fn the_plural_scan_sees_what_it_should() {
    assert!(fixed_plural("{{ n }} commits").is_some());
    assert!(fixed_plural("<b>{{ n }}</b> authors").is_some());
    assert!(fixed_plural("{{ n }} commit").is_none());
    assert!(fixed_plural("{{ t.commits_noun }}").is_none());
    assert!(fixed_plural("{{ a }}\n  <td>files</td>").is_none());
}
