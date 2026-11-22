//! Every chart mark's hover names what it is, its value with a unit and, where it is part of a
//! column, its share and the column total. Charts never write their own `data-tip`: one builder
//! writes them all, so no chart can drop the total again.

use std::path::Path;

/// The `data-tip` values inside the `<svg>` charts and share bars of `body`.
pub(crate) fn chart_tips(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut scopes: Vec<&str> = body
        .split("<svg class=\"chart")
        .skip(1)
        .map(|c| c.split("</svg>").next().unwrap_or_default())
        .collect();
    scopes.extend(
        body.split("class=\"sharebar\"")
            .skip(1)
            .map(|c| c.split("</div>").next().unwrap_or_default()),
    );
    for c in scopes {
        for t in c.split("data-tip=\"").skip(1) {
            out.push(t.split('"').next().unwrap_or_default().to_owned());
        }
    }
    out
}

/// What a tip lacks: a number with a unit, and for a part of a column its share and total.
pub(crate) fn lacks(tip: &str) -> Option<&'static str> {
    if !tip.chars().any(|c| c.is_ascii_digit()) {
        return Some("no number");
    }
    if !tip.contains('%') {
        return Some("no share");
    }
    if !(tip.contains(" of ") || tip.contains(", ")) {
        return Some("no total");
    }
    None
}

#[test]
fn a_tip_naming_only_its_mark_is_caught() {
    // The languages area once named the language alone, the share bar a name and a share.
    assert_eq!(lacks("Rust"), Some("no number"));
    assert_eq!(lacks("Rust 62.0%"), Some("no total"));
    assert_eq!(lacks("Rust, 4 May 2026: 41,230 lines (62%) of 66,500"), None);
    assert_eq!(lacks("Q1 2026: 40 commits: feat 30 (75%), fix 10 (25%)"), None);
}

/// Only the builder writes a chart's `data-tip`.
#[test]
fn charts_write_no_tip_by_hand() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut bad = Vec::new();
    for f in ["charts.rs", "languages/render.rs"] {
        let text = std::fs::read_to_string(root.join(f)).expect("read");
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        if code.contains("data-tip=") {
            bad.push(f);
        }
    }
    assert!(bad.is_empty(), "hand-written chart tip in {bad:?}");
}

/// A chart template prints a tip the builder made and writes none itself: every `data-tip`
/// in a `chart_*.svg` or the languages body holds one expression and no literal text.
#[test]
fn chart_templates_print_built_tips_only() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut seen = 0;
    let mut bad = Vec::new();
    for e in std::fs::read_dir(dir).expect("templates").flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let svg =
            std::path::Path::new(&name).extension().is_some_and(|x| x.eq_ignore_ascii_case("svg"));
        let chart = name.starts_with("chart_") && svg;
        if !(chart || name == "languages_body.html") {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).expect("read");
        for v in text.split("data-tip=\"").skip(1) {
            seen += 1;
            let v = v.split('"').next().unwrap_or_default();
            let one = v.starts_with("{{ ") && v.ends_with(" }}") && v.matches("{{").count() == 1;
            if !one {
                bad.push(format!("{name}: {v}"));
            }
        }
    }
    assert!(seen >= 5, "the scan found the chart tips: {seen}");
    assert!(bad.is_empty(), "literal text in a chart tip: {bad:?}");
}
