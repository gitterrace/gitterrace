//! Things done once stay once: a second copy of the git test helper, or a second spelling of
//! the default address, fails here instead of drifting.

use std::{
    fs,
    path::{Path, PathBuf},
};

fn sources(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            if name != "target" && name != "node_modules" && name != ".git" {
                sources(&p, exts, out);
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| exts.contains(&x))
        {
            out.push(p);
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Offending `file:line` entries for `needle` in files under `dirs`, skipping `allowed` paths.
fn hits(dirs: &[&str], exts: &[&str], allowed: &[&str], bad: impl Fn(&str) -> bool) -> Vec<String> {
    let mut files = Vec::new();
    for d in dirs {
        sources(&root().join(d), exts, &mut files);
    }
    assert!(!files.is_empty(), "no sources found");
    let mut out = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(root())
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        if allowed.iter().any(|a| rel.contains(a)) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            if bad(line) {
                out.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    out
}

/// A test helper that runs git belongs to `gitterrace-testkit`. Matches `fn git(` and
/// `fn git_at(`, and a raw `Command::new("git")`, so a differently named copy is caught too.
#[test]
fn no_git_test_helper_outside_the_testkit() {
    let defs = [concat!("fn ", "git("), concat!("fn ", "git_at(")];
    let raw = concat!("Command::new(\"", "git\")");
    let found = hits(
        &["crates", "xtask"],
        &["rs"],
        &["gitterrace-testkit/", "crates/gitterrace/build.rs"],
        |l| defs.iter().any(|d| l.contains(d)) || l.contains(raw),
    );
    assert!(
        found.is_empty(),
        "use gitterrace_testkit::git instead:\n{}",
        found.join("\n")
    );
}

/// The default address is written once, as `DEFAULT_ADDR` in the config schema. Production reads
/// it from there, tests from the testkit. Digits on either side mean a different number.
#[test]
fn default_address_is_not_respelled() {
    let port = concat!("42", "42");
    let found = hits(
        &["crates", "scripts", "xtask"],
        &["rs", "sh", "py", "mjs", "js"],
        &[
            "gitterrace-config/src/schema.rs",
            "gitterrace-testkit/tests/",
        ],
        |l| {
            l.match_indices(port).any(|(i, _)| {
                let before = l[..i].chars().next_back();
                let after = l[i + 4..].chars().next();
                !before.is_some_and(|c| c.is_ascii_alphanumeric())
                    && !after.is_some_and(|c| c.is_ascii_alphanumeric())
            })
        },
    );
    assert!(
        found.is_empty(),
        "derive it from DEFAULT_ADDR:\n{}",
        found.join("\n")
    );
}
