//! Every author avatar is the one `_avatar.html` macro writes: the initial on a tint from the
//! name's hue. A hand-written copy drifts in its tint step or loses the escaping of the initial.

use std::path::Path;

/// Spellings of the avatar's class in a template, a Rust string or a script.
const OWNED: &[&str] = &["class=\"av ", "class=\\\"av ", "class=\"av\"", "'av h"];

/// The avatar markup `code` writes by hand. Rust test modules may name the class in asserts.
fn loose_avatars(code: &str) -> usize {
    let code = code.split("#[cfg(test)]").next().unwrap_or_default();
    OWNED.iter().map(|m| code.matches(m).count()).sum()
}

#[test]
fn a_hand_written_avatar_is_caught() {
    assert_eq!(
        loose_avatars("<span class=\"av h{{ p.hue / 10 }}\">x</span>"),
        1
    );
    assert_eq!(loose_avatars("format!(\"<span class=\\\"av h{}\\\">\")"), 1);
    assert_eq!(
        loose_avatars("fn a() {}\n#[cfg(test)]\nmod t { \"class=\\\"av \" }"),
        0
    );
    assert_eq!(
        loose_avatars("{% call avatars::avatar(p.hue, p.initial) %}{% endcall %}"),
        0
    );
}

/// Each file under `dir` with one of `exts`, recursively.
fn files(dir: &Path, exts: &[&str], out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, exts, out);
        } else if p.extension().is_some_and(|x| exts.iter().any(|w| x == *w)) {
            out.push(p);
        }
    }
}

#[test]
fn only_the_avatar_macro_writes_an_avatar() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all = Vec::new();
    for d in ["templates", "src", "static"] {
        files(&root.join(d), &["html", "rs", "js"], &mut all);
    }
    let bad: Vec<String> = all
        .iter()
        .filter(|p| !p.ends_with("_avatar.html"))
        .filter(|p| loose_avatars(&std::fs::read_to_string(p).unwrap_or_default()) > 0)
        .map(|p| p.display().to_string())
        .collect();
    assert!(bad.is_empty(), "an avatar written by hand: {bad:?}");
}
