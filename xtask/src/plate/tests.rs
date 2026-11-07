use super::*;

const DARK: &str =
    "<svg><title>gitterrace</title><g><path fill=\"#e3ecf6\" d=\"M0 0\"/></g></svg>\n";

/// The mark's paths appear once per file and the plate color is the constant, so a changed
/// color changes every plated output.
#[test]
fn plate_color_reaches_every_output() {
    let a = plated(DARK, PLATE_COLOR).expect("plated");
    let b = plated(DARK, "#000000").expect("plated");
    assert!(a.contains(PLATE_COLOR) && a.contains("M0 0"));
    assert_ne!(a, b);
    let fa = favicon(DARK, PLATE_COLOR).expect("favicon");
    let fb = favicon(DARK, "#000000").expect("favicon");
    assert_ne!(fa, fb);
    assert_ne!(favicon_link(&fa), favicon_link(&fb));
}

/// The favicon drops the plate in dark mode and keeps the mark's own colors.
#[test]
fn favicon_swaps_plate_for_bare_mark_in_dark_mode() {
    let f = favicon(DARK, PLATE_COLOR).expect("favicon");
    assert!(f.contains("prefers-color-scheme:dark"));
    assert!(f.contains(".p{display:none}"));
}

/// The data URI holds no raw character that ends an attribute or starts markup.
#[test]
fn favicon_link_is_one_safe_attribute() {
    let l = favicon_link(&favicon(DARK, PLATE_COLOR).expect("favicon"));
    let uri = l
        .trim_end()
        .trim_start_matches("<link rel=\"icon\" href=\"");
    let uri = uri.strip_suffix("\">").expect("closing");
    assert!(!uri.contains(['"', '<', '>', '#']));
}

/// The drift check: the tree matches the generator, and the same check goes red when the
/// plate color differs (a copy of the generator with another color).
#[test]
fn tree_matches_generator_and_a_changed_color_is_stale() {
    use crate::out::{Mode, apply};
    let root = crate::repo_root();
    let ok = apply(&root, outs(&root, PLATE_COLOR).expect("outs"), Mode::Check).expect("check");
    assert!(ok.is_empty(), "stale: {ok:?}");
    let red = apply(&root, outs(&root, "#ff0000").expect("outs"), Mode::Check).expect("check");
    assert_eq!(red.len(), 3, "every plated file must notice: {red:?}");
}

/// Every path the script `scripts/theme-logo-gen.py` writes, as it lists them.
fn script_outputs() -> Vec<String> {
    let root = crate::repo_root();
    let run = std::process::Command::new("python3")
        .args(["scripts/theme-logo-gen.py", "--list-outputs"])
        .current_dir(root)
        .output()
        .expect("run scripts/theme-logo-gen.py");
    assert!(run.status.success(), "--list-outputs failed");
    String::from_utf8(run.stdout)
        .expect("utf8")
        .lines()
        .map(str::to_string)
        .collect()
}

/// A file is written by one generator. Two writers with different output leave whichever ran
/// last in place, so the other one's check fails.
#[test]
fn no_file_has_two_writers() {
    let root = crate::repo_root();
    let ours: Vec<String> = outs(&root, PLATE_COLOR)
        .expect("outs")
        .iter()
        .map(|o| match o {
            Out::File { path, .. } | Out::Bytes { path, .. } | Out::Block { path, .. } => {
                path.clone()
            }
        })
        .collect();
    let theirs = script_outputs();
    let both: Vec<&String> = theirs.iter().filter(|p| ours.contains(p)).collect();
    assert!(
        both.is_empty(),
        "written by xtask and by theme-logo-gen.py: {both:?}"
    );
}
