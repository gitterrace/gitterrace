//! Class guards over every prose doc: a name the docs print must still exist in the code.
//!
//! Prose keeps decisions, and decisions name things: a config key, a repo path, a make target,
//! a TOML example. Each is checked against its owner (the settings table, the tree, the
//! Makefile, the loaders), outside the GENERATED blocks, which `cargo xtask generate` keeps
//! true. A key renamed in code while a doc keeps the old name fails the first check.

#![expect(
    clippy::expect_used,
    reason = "the module only builds for tests, where a failure is a panic"
)]

use std::collections::BTreeSet;
use std::path::Path;

use regex::Regex;

/// Top-level folders a doc may name by path. They are checked even after the folder is gone,
/// so deleting or renaming one cannot hide a stale path.
const ROOTS: [&str; 8] =
    ["crates/", "docs/", "scripts/", "xtask/", "vendor/", "deploy/", "schwarzwald/", ".github/"];

/// What the docs are checked against.
struct Owners<'a> {
    /// Every `section.key` of the settings table.
    keys: BTreeSet<String>,
    /// The sections of the settings table.
    sections: BTreeSet<String>,
    /// Every target of the Makefile.
    targets: BTreeSet<String>,
    /// The repository root, for paths.
    root: &'a Path,
}

impl<'a> Owners<'a> {
    fn new(root: &'a Path, makefile: &str) -> Self {
        let keys: BTreeSet<String> =
            gitterrace_config::keys().iter().map(|k| k.id.to_owned()).collect();
        let sections = gitterrace_config::keys().iter().map(|k| k.section.to_owned()).collect();
        let rule = Regex::new(r"(?m)^([A-Za-z0-9_. -]+):").expect("static regex");
        let targets = rule
            .captures_iter(makefile)
            .flat_map(|c| c[1].split_whitespace().map(str::to_owned).collect::<Vec<_>>())
            .collect();
        Self { keys, sections, targets, root }
    }
}

/// The page without its GENERATED blocks.
fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("<!-- BEGIN GENERATED: ") {
            inside = true;
        } else if line.starts_with("<!-- END GENERATED: ") {
            inside = false;
        } else if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The fenced blocks of `text` as (info string, body), and the text without them.
fn fences(text: &str) -> (Vec<(String, String)>, String) {
    let (mut blocks, mut rest) = (Vec::new(), String::new());
    let mut open: Option<(String, String)> = None;
    for line in text.lines() {
        if let Some(info) = line.trim_start().strip_prefix("```") {
            match open.take() {
                Some(b) => blocks.push(b),
                None => open = Some((info.trim().to_owned(), String::new())),
            }
        } else if let Some((_, body)) = open.as_mut() {
            body.push_str(line);
            body.push('\n');
        } else {
            rest.push_str(line);
            rest.push('\n');
        }
    }
    (blocks, rest)
}

fn is_path(word: &str) -> bool {
    !word.contains(['*', ' ', '<', '{', '$', ':']) && ROOTS.iter().any(|r| word.starts_with(r))
}

/// Why `page` disagrees with the code, one line each; empty when it is true.
fn problems(name: &str, page: &str, o: &Owners<'_>) -> Vec<String> {
    let mut bad = Vec::new();
    let (blocks, text) = fences(&prose(page));
    let key = Regex::new(r"^([a-z_]+)\.([a-z_]+)$").expect("static regex");
    let make = Regex::new(r"\bmake ([a-z][a-z0-9-]*)").expect("static regex");
    let spans = text.split('`').skip(1).step_by(2);
    let lines = blocks.iter().filter(|(i, _)| matches!(i.as_str(), "" | "sh" | "bash" | "console"));
    let from_fences = lines.flat_map(|(_, b)| b.lines().map(str::to_owned)).collect::<Vec<_>>();
    for span in spans.clone() {
        if let Some(c) = key.captures(span)
            && o.sections.contains(&c[1])
            && !o.keys.contains(span)
        {
            bad.push(format!("{name}: `{span}` is not a config key"));
        }
        let path = span.split('#').next().unwrap_or(span).trim_end_matches('/');
        if is_path(span) && !o.root.join(path).exists() {
            bad.push(format!("{name}: `{span}` is not in the repository"));
        }
    }
    for text in spans.map(str::to_owned).chain(from_fences) {
        for c in make.captures_iter(&text) {
            if !o.targets.contains(&c[1]) {
                bad.push(format!("{name}: `make {}` is not a Makefile target", &c[1]));
            }
        }
    }
    for (i, (info, body)) in blocks.iter().enumerate() {
        if info == "toml" {
            bad.extend(toml_problem(body).map(|e| format!("{name}: toml block {i}: {e}")));
        }
    }
    bad
}

/// Why a toml fence does not load: its text is a config file (`[server]`, no stray key) or an
/// actions file (loaded with its programs stubbed), whichever its tables say.
fn toml_problem(block: &str) -> Option<String> {
    let actions = ["[[action]]", "[[send]]", "[[script]]"].iter().any(|t| block.contains(t));
    if !actions {
        return match gitterrace_config::parse(block) {
            Ok(l) => l.warnings.first().cloned(),
            Err(e) => Some(e.to_string()),
        };
    }
    let home = tempfile::tempdir().ok()?;
    let mut text =
        if block.contains("version") { block.to_owned() } else { format!("version = 1\n{block}") };
    let programs: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("command"))
        .filter_map(|l| l.split('"').nth(1))
        .map(str::to_owned)
        .collect();
    for prog in programs {
        let abs = match prog.strip_prefix("~/") {
            Some(rest) => home.path().join(rest),
            None if prog.contains('/') => continue,
            None => home.path().join("bin").join(&prog),
        };
        stub(&abs);
        if !prog.contains('/') {
            text = text.replace(&format!("[\"{prog}\""), &format!("[\"{}\"", abs.display()));
        }
    }
    gitterrace_actions::Config::parse(&text, home.path()).err().map(|e| e.to_string())
}

fn stub(p: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(p, "#!/bin/sh\n").ok();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).ok();
}

#[cfg(test)]
mod tests {
    use super::{Owners, problems};

    const MAKEFILE: &str = ".PHONY: a\ncheck: fmt lint\nfmt:\n\tcargo fmt\nPWCLI = x\n";

    fn root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    fn found(page: &str) -> Vec<String> {
        let root = root();
        problems("t.md", page, &Owners::new(&root, MAKEFILE))
    }

    /// Every prose doc, outside its generated blocks, names only what the code still has.
    #[test]
    fn every_prose_doc_names_only_what_exists() {
        let root = root();
        let makefile = std::fs::read_to_string(root.join("Makefile")).expect("Makefile");
        let o = Owners::new(&root, &makefile);
        let mut pages = vec!["README.md".to_owned(), "CONTRIBUTING.md".to_owned()];
        for e in std::fs::read_dir(root.join("docs")).expect("docs").flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(".md") {
                pages.push(format!("docs/{n}"));
            }
        }
        assert!(pages.len() > 8, "found only {pages:?}");
        let bad: Vec<String> = pages
            .iter()
            .flat_map(|p| {
                let text = std::fs::read_to_string(root.join(p)).expect("page");
                problems(p, &text, &o)
            })
            .collect();
        assert!(bad.is_empty(), "docs name what the code lacks:\n{}", bad.join("\n"));
    }

    /// The defect that happened: code renames a key, the docs keep the old name.
    #[test]
    fn a_key_renamed_in_code_while_the_docs_keep_the_old_name_is_found() {
        let root = root();
        let mut o = Owners::new(&root, MAKEFILE);
        let page = "Raise `limits.unindexed_commits` for a long history.\n";
        assert!(problems("t.md", page, &o).is_empty(), "the key exists today");
        o.keys.remove("limits.unindexed_commits");
        o.keys.insert("limits.unindexed_commit_cap".to_owned());
        let bad = problems("t.md", page, &o);
        assert_eq!(bad, ["t.md: `limits.unindexed_commits` is not a config key"], "{bad:?}");
        assert_eq!(found("Set `limits.never_was` high.\n").len(), 1);
    }

    #[test]
    fn a_current_config_key_and_a_file_name_pass() {
        assert!(found("Set `display.diff_palette`, in `config.toml`, at `a.b`.\n").is_empty());
    }

    #[test]
    fn a_missing_repo_path_is_found_and_a_present_one_passes() {
        assert_eq!(found("See `docs/gone.md` and `crates/nope/src/lib.rs`.\n").len(), 2);
        assert!(found("See `docs/using.md`, `docs/*.md` and `crates/`.\n").is_empty());
    }

    #[test]
    fn a_missing_make_target_is_found_in_prose_and_in_a_shell_fence() {
        assert!(found("Run `make check` and `make fmt`.\n").is_empty());
        let bad = found("Run `make nothing`.\n\n```sh\nmake gone\nmake fmt\n```\n");
        assert_eq!(bad.len(), 2, "{bad:?}");
    }

    #[test]
    fn a_toml_block_that_does_not_load_is_found() {
        let bad = found("```toml\n[server]\nadress = \"x\"\n```\n");
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert_eq!(found("```toml\n[server\n```\n").len(), 1);
        let act = "```toml\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\ncommand = \
                   [\"nope\"]\nbogus = 1\n```\n";
        assert_eq!(found(act).len(), 1);
        assert!(found("```toml\n[server]\nis_public = true\n```\n").is_empty());
    }

    #[test]
    fn a_generated_block_is_left_to_the_generator() {
        let page =
            "<!-- BEGIN GENERATED: x -->\n`limits.gone` `make gone`\n<!-- END GENERATED: x -->\n";
        assert!(found(page).is_empty());
    }
}
