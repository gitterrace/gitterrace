//! A class selector in the static CSS that nothing emits is dead weight: it fails here.

use std::{fs, path::Path};

/// Families built as `format!("w{n}")` and the like, so their members are not spelled out in any
/// source. The prefix is checked instead.
const RAMPS: [&str; 7] = ["w", "h", "lw", "kw", "m", "s", "c"];

/// Classes written by the markdown renderer, not by this repo: comrak names the footnote ones,
/// `alert_open` and `levels.rs` build the alert and heading ones with `format!`.
const RENDERED: [&str; 4] = [
    "footnote-definition",
    "markdown-alert-",
    "markdown-h",
    "footnote-definition-label",
];

fn read_tree(dir: &Path, exts: &[&str], out: &mut String) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "vendor") {
                read_tree(&path, exts, out);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push_str(&fs::read_to_string(&path).unwrap_or_default());
            out.push('\n');
        }
    }
}

#[test]
fn every_css_class_is_emitted_somewhere() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut css = String::new();
    read_tree(&root.join("static"), &["css"], &mut css);
    let mut code = String::new();
    read_tree(&root.join("templates"), &["html", "svg", "txt"], &mut code);
    read_tree(&root.join("src"), &["rs"], &mut code);
    read_tree(&root.join("static"), &["js"], &mut code);
    let dead = gitterrace_testkit::css::dead_classes(&css, &code, &RAMPS, &RENDERED);
    assert!(
        dead.is_empty(),
        "classes in static CSS that nothing emits: {dead:?}"
    );
}
