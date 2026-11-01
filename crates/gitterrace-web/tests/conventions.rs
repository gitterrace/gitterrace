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

/// Nouns that must never follow a number fixed in the plural: `{{ n }} commits` says
/// "1 commits", `{n} hours` says "1 hours". Render the pair with `fmt::count` or `fmt::noun`
/// (cards: `svg::count`). Time units count too.
const COUNTED_NOUNS: [&str; 26] = [
    "commits",
    "authors",
    "files",
    "lines",
    "repositories",
    "contributors",
    "changes",
    "matches",
    "branches",
    "tags",
    "releases",
    "people",
    "hunks",
    "results",
    "times",
    "seconds",
    "minutes",
    "hours",
    "days",
    "nights",
    "weekdays",
    "weekends",
    "weeks",
    "months",
    "years",
    "streaks",
];

/// The counted noun at the start of `s` after optional spaces, if any.
fn counted_noun(s: &str) -> Option<&str> {
    let word = s.trim_start_matches([' ', '\t', '\n']);
    if word.len() == s.len() {
        return None;
    }
    let end = word
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(word.len());
    COUNTED_NOUNS.contains(&&word[..end]).then(|| &word[..end])
}

/// `}}` or `}}</b>`, whitespace, then a counted noun, as a word.
fn fixed_plural(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(at) = rest.find("}}") {
        rest = &rest[at + 2..];
        let tail = rest.strip_prefix("</b>").unwrap_or(rest);
        if let Some(noun) = counted_noun(tail) {
            return Some(format!("}}}} {noun}"));
        }
    }
    None
}

/// In Rust source: a format placeholder (`{n}`, `{}`, `{x:>3}`) then a counted noun, as in
/// `format!("{n} hours")`. In JS: a closing `+ "` then a counted noun, as in `n + " hours"`.
fn fixed_plural_in_code(src: &str) -> Option<String> {
    for line in src.lines() {
        let code = line.trim_start();
        if code.starts_with("//") || code.starts_with("assert") {
            continue;
        }
        let mut rest = line;
        while let Some(at) = rest.find('}') {
            let before = &rest[..at];
            rest = &rest[at + 1..];
            // `{}`/`{name}`/`{n:>3}` only: not `}}` escapes, not a closing block brace.
            let open = before.rfind('{');
            let placeholder = open.is_some_and(|o| {
                before[o + 1..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.:<>^".contains(c))
            });
            if placeholder
                && !rest.starts_with('}')
                && let Some(noun) = counted_noun(rest)
            {
                return Some(format!("{{..}} {noun}: {}", line.trim()));
            }
        }
        for quote in ["+ \"", "+ '"] {
            let mut rest = line;
            while let Some(at) = rest.find(quote) {
                rest = &rest[at + quote.len()..];
                if let Some(noun) = counted_noun(rest) {
                    return Some(format!("+ \" {noun}: {}", line.trim()));
                }
            }
        }
    }
    None
}

/// The text sources a page or card is built from: web and cards Rust, and the static JS.
/// An unreadable directory yields nothing, so the test below asserts it found sources.
fn code_sources() -> Vec<std::path::PathBuf> {
    let web = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in [
        web.join("src"),
        web.join("static"),
        web.join("../gitterrace-cards/src"),
    ] {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|e| e == "rs" || e == "js") {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn code_never_pairs_a_number_with_a_fixed_plural() {
    let sources = code_sources();
    assert!(sources.len() > 20, "found only {} sources", sources.len());
    for path in sources {
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        // Unit tests at the bottom may spell out expected text; the guard is for page text.
        let src = src.split("#[cfg(test)]").next().unwrap_or(&src);
        if let Some(hit) = fixed_plural_in_code(src) {
            panic!(
                "{} builds `{hit}`: use fmt::count / fmt::noun (cards: svg::count)",
                path.display()
            );
        }
    }
}

#[test]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "the inputs are source text the scan reads, not format strings"
)]
fn the_code_plural_scan_sees_what_it_should() {
    assert!(fixed_plural_in_code(r#"format!("{n} hours")"#).is_some());
    assert!(fixed_plural_in_code(r#"format!("{} times", x)"#).is_some());
    assert!(fixed_plural_in_code(r#"format!("{d:>3} days since")"#).is_some());
    assert!(fixed_plural_in_code(r#"var t = n + " commits";"#).is_some());
    assert!(fixed_plural_in_code(r#"format!("{} in 26 weeks", c)"#).is_none());
    assert!(fixed_plural_in_code(r#"format!("{n} {unit}{} ago", s)"#).is_none());
    assert!(fixed_plural_in_code("if x { y } days").is_none());
    assert!(fixed_plural_in_code(r#"format!("{{n}} commits")"#).is_none());
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
