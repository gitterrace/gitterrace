//! Every row of headline figures comes from the `kpis` macros in `_kpis.html`, so the figure,
//! its label and the warning color stay one markup. A template or Rust source that writes the
//! row's class by hand drifts from it.
#![expect(clippy::expect_used, reason = "an unreadable source tree should panic")]

use std::path::Path;

/// The row's class, as a template or a Rust string writes it.
const ROW: [&str; 2] = ["class=\"kpis\"", "class=\\\"kpis\\\""];

/// Whether `code` writes a figure row by hand.
fn writes_row(code: &str) -> bool {
    ROW.iter().any(|m| code.contains(m))
}

fn walk(dir: &Path, bad: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).expect("dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, bad);
            continue;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
        if name
            .as_deref()
            .is_some_and(|n| n == "_kpis.html" || n.ends_with("tests.rs"))
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        if writes_row(code) {
            bad.push(p.display().to_string());
        }
    }
}

#[test]
fn a_hand_written_figure_row_is_caught() {
    assert!(writes_row(r#"<div class="kpis"><div><b>3</b></div></div>"#));
    assert!(writes_row(r#"format!("<div class=\"kpis\">{x}</div>")"#));
    assert!(!writes_row(r"{% call kp::kpis(kpis) %}{% endcall %}"));
}

#[test]
fn only_the_kpis_macro_writes_a_figure_row() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    walk(&root.join("templates"), &mut bad);
    walk(&root.join("src"), &mut bad);
    assert!(bad.is_empty(), "hand-written figure row in {bad:?}");
}
