//! Markup belongs in templates. Rust code that builds HTML with `format!`, `write!` or
//! `writeln!` is counted, and the count is the `web_format_html` entry of the ratchet registry,
//! which may only fall. A new builder fails here until the registry grows, and the registry
//! check refuses that growth.
#![expect(clippy::expect_used, reason = "an unreadable source tree should panic")]

use std::path::Path;

/// Macros whose first string literal holds a tag.
const MACROS: [&str; 3] = ["format!(", "write!(", "writeln!("];

/// Number of HTML-building macro calls in `code`.
fn count_in(code: &str) -> usize {
    let mut n = 0;
    for m in MACROS {
        let mut rest = code;
        while let Some(i) = rest.find(m) {
            let after = &rest[i + m.len()..];
            // A `write!` match inside `writeln!(` is skipped: "writeln!(" never contains "write!(".
            let lit = after.find('"').and_then(|q| {
                let body = &after[q + 1..];
                body.find('"').map(|e| &body[..e])
            });
            let args_start = after.find([')', ';']).unwrap_or(after.len());
            if let Some(lit) = lit.filter(|_| after.find('"').is_some_and(|q| q < args_start)) {
                let b = lit.as_bytes();
                if b.windows(2)
                    .any(|w| w[0] == b'<' && (w[1].is_ascii_alphabetic() || w[1] == b'/'))
                {
                    n += 1;
                }
            }
            rest = after;
        }
    }
    n
}

fn walk(dir: &Path, n: &mut usize) {
    for e in std::fs::read_dir(dir).expect("dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, n);
        } else if p.extension().is_some_and(|x| x == "rs")
            && !p
                .file_name()
                .is_some_and(|f| f.to_string_lossy().ends_with("tests.rs"))
        {
            let text = std::fs::read_to_string(&p).expect("read");
            *n += count_in(text.split("#[cfg(test)]").next().unwrap_or_default());
        }
    }
}

/// The `web_format_html` count of the ratchet registry.
fn registered(root: &Path) -> usize {
    let reg = std::fs::read_to_string(root.join("schwarzwald/config/ratchet.toml")).expect("reg");
    reg.lines()
        .find_map(|l| l.strip_prefix("web_format_html = "))
        .and_then(|v| v.trim().parse().ok())
        .expect("web_format_html in [counts]")
}

#[test]
fn html_built_in_rust_matches_the_ratchet() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut n = 0;
    walk(&crate_dir.join("src"), &mut n);
    let want = registered(&crate_dir.join("../.."));
    assert_eq!(
        n, want,
        "HTML built with format!/write! in src: {n}, registry web_format_html: {want}. \
         Move new markup into a template, or lower the registry when one leaves."
    );
}

#[test]
fn the_counter_sees_tags_only() {
    assert_eq!(count_in(r#"format!("<a href={x}>")"#), 1);
    assert_eq!(count_in(r#"write!(out, "</td>")"#), 1);
    assert_eq!(count_in(r#"writeln!(out, "<li>")"#), 1);
    assert_eq!(count_in(r#"format!("{a} < {b}")"#), 0);
    assert_eq!(count_in(r#"format!("{x}")"#), 0);
}
