//! AGENTS.md rules that a grep can hold: they fail here instead of in review.

use std::{fs, path::Path};

fn rust_sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `use super::` is allowed only inside `mod tests`, which is last in the file, so
/// every hit must come after the `mod tests` line.
#[test]
fn use_super_only_in_tests() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_sources(&crates, &mut files);
    assert!(!files.is_empty(), "no sources found");
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let tests_at = text
            .lines()
            .position(|l| l.starts_with("mod tests"))
            .unwrap_or(usize::MAX);
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            assert!(
                !(t.starts_with("use super::") && i < tests_at),
                "{}:{} uses `use super::` outside mod tests, use a crate:: path",
                file.display(),
                i + 1
            );
        }
    }
}

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
