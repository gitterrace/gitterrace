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
/// The build script and the test of its stamp module are exempt, like in the ast-grep rule
/// `no-git-subprocess`: the stamp runs git at build time and its test must run the same way.
#[test]
fn no_git_test_helper_outside_the_testkit() {
    let defs = [concat!("fn ", "git("), concat!("fn ", "git_at(")];
    let raw = concat!("Command::new(\"", "git\")");
    let found = hits(
        &["crates", "xtask"],
        &["rs"],
        &[
            "gitterrace-testkit/",
            "crates/gitterrace/build.rs",
            "crates/gitterrace/stamp.rs",
            "crates/gitterrace/tests/stamp.rs",
        ],
        |l| defs.iter().any(|d| l.contains(d)) || l.contains(raw),
    );
    assert!(
        found.is_empty(),
        "use gitterrace_testkit::git instead:\n{}",
        found.join("\n")
    );
}

/// Limits that became settings. Each must stay out of the code as a raw constant: its value
/// lives in the config schema now, and a copy would silently stop following the setting.
const MOVED_TO_CONFIG: [&str; 18] = [
    "MAX_PAGES",
    "MAX_FILES",
    "MAX_DIFF_BYTES",
    "MAX_BLOB_LINES",
    "HIGHLIGHT_LIMIT",
    "MAX_PER_FILE",
    "MAX_COMMITS",
    "MAX_PAGE",
    "MAX_ENTRIES",
    "MAX_PREVIEW_BYTES",
    "PUBLIC_TIMEOUT",
    "MAX_OUTPUT_BYTES",
    "MAX_BLOB_BYTES",
    "MAX_BLAME_LINES",
    "MAX_DIFF_LINES",
    "MAX_LOG_SKIP",
    "MAX_FRAGMENTS",
    "MAX_BUCKETS_SETTING",
];

fn const_name(line: &str) -> Option<&str> {
    let rest = line.trim_start();
    let rest = ["pub(crate) ", "pub(super) ", "pub "]
        .iter()
        .find_map(|p| rest.strip_prefix(p))
        .unwrap_or(rest);
    let rest = rest
        .strip_prefix("const ")
        .or_else(|| rest.strip_prefix("static "))?;
    rest.split([':', ' ']).next()
}

#[test]
fn moved_limits_never_come_back_as_constants() {
    let found = hits(&["crates"], &["rs"], &[], |l| {
        const_name(l).is_some_and(|n| MOVED_TO_CONFIG.contains(&n))
    });
    assert!(
        found.is_empty(),
        "a limit that is a setting is a raw constant again:\n{}",
        found.join("\n")
    );
}

/// Every limit-shaped constant is classified: its doc says why it is not configurable, or names
/// the setting it is the default of. Catches a new raw cap added without that decision.
#[test]
fn every_limit_constant_is_classified() {
    let shaped = |n: &str| {
        n.starts_with("MAX_")
            || n.ends_with("_MAX")
            || n.ends_with("_CAP")
            || n.contains("LIMIT")
            || n.ends_with("_TIMEOUT")
            || n == "MAX"
            || n == "CAP"
    };
    let mut files = Vec::new();
    sources(&root().join("crates"), &["rs"], &mut files);
    let mut bad = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(root())
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.contains("/tests/") || rel.ends_with("tests.rs") || !rel.contains("/src/") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        // Test modules sit at the end of a file; their constants are fixtures, not limits.
        let end = lines
            .iter()
            .position(|l| l.trim() == "mod tests {")
            .unwrap_or(lines.len());
        for (i, l) in lines[..end].iter().enumerate() {
            let Some(name) = const_name(l).filter(|n| shaped(n)) else {
                continue;
            };
            // The doc block above, across a run of grouped constants and attributes.
            let mut above = Vec::new();
            for prev in lines[..i].iter().rev() {
                let t = prev.trim();
                if t.starts_with("///") || t.starts_with("#[") || const_name(t).is_some() {
                    above.push(t);
                } else {
                    break;
                }
            }
            above.reverse();
            let doc = above.join(" ").replace("///", " ");
            let doc = doc.split_whitespace().collect::<Vec<_>>().join(" ");
            if !(doc.contains("Not configurable")
                || doc.contains("`limits.")
                || doc.contains("`api.")
                || doc.contains("`server."))
            {
                bad.push(format!("{rel}:{}: {name}", i + 1));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "limit constants without a classification:\n{}",
        bad.join("\n")
    );
}
