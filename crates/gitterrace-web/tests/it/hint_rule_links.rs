//! A chart whose numbers can surprise says why in its own hint line, so the reason reads
//! offline, and never by a link: the explanation is the sentence.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::path::Path;

/// The template and the subtlety its hint states inline.
const SUBTLETIES: [(&str, &str); 5] = [
    ("stats.html", "Merges"),
    ("contributors.html", ".mailmap"),
    ("ownership.html", "renamed file"),
    ("hotspots.html", "renamed file"),
    ("frequency_section.html", "Merges"),
];

fn templates() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).expect("templates").flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        out.push((name, std::fs::read_to_string(e.path()).unwrap_or_default()));
    }
    out
}

/// The text of every `<p class="hint">` in `code`.
fn hints(code: &str) -> Vec<&str> {
    code.match_indices("<p class=\"hint\">")
        .map(|(i, m)| {
            let rest = &code[i + m.len()..];
            &rest[..rest.find("</p>").unwrap_or(rest.len())]
        })
        .collect()
}

#[test]
fn surprising_counts_are_explained_in_their_hint_and_no_hint_links_off_the_machine() {
    let all = templates();
    for (file, phrase) in SUBTLETIES {
        let (_, code) = all.iter().find(|(n, _)| n == file).expect(file);
        assert!(
            hints(code).iter().any(|h| h.contains(phrase)),
            "{file}: no hint says \"{phrase}\" inline"
        );
    }
    for (file, code) in all {
        for hint in hints(&code) {
            assert!(!hint.contains("href=\"http"), "{file}: a hint links off the machine: {hint}");
        }
    }
}
