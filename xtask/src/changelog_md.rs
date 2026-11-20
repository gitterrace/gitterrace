//! The `[Unreleased]` section of `CHANGELOG.md`: what changed since the last release, written
//! by git-cliff from the commits with the release's own `cliff.toml`, so the changelog page has
//! content between releases and the release step only names the version. A released section is
//! never touched here: `make changelog` writes it at the tag, and the changelog lint holds it.
//!
//! The section is as of the last `make docs-gen`. Every commit moves it, the one that writes it
//! included, so `check` keeps the text in the file instead of asking for a regenerate on every
//! commit; the changelog lint leaves Unreleased out for the same reason.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result};

use crate::out::Out;

const FILE: &str = "CHANGELOG.md";
/// The line width the repository's markdown wraps at.
const WIDTH: usize = 80;

const BEGIN: &str = "<!-- BEGIN GENERATED: unreleased -->\n";
const END: &str = "<!-- END GENERATED: unreleased -->";

/// The Unreleased block. With `refresh` it is written from the commits; without it, and when
/// git-cliff is not installed, it is the text the file holds.
pub fn render(root: &Path, refresh: bool) -> Result<Vec<Out>> {
    let current = || -> Result<Vec<Out>> {
        let text = std::fs::read_to_string(root.join(FILE)).context("reading CHANGELOG.md")?;
        let body =
            text.split_once(BEGIN).and_then(|(_, r)| r.split_once(END)).map_or("", |(b, _)| b);
        Ok(vec![Out::block(FILE, "unreleased", body)])
    };
    if !refresh {
        return current();
    }
    let file = std::fs::read_to_string(root.join(FILE)).context("reading CHANGELOG.md")?;
    let Ok(out) = Command::new("git-cliff")
        .args(["--unreleased", "--strip", "all"])
        .current_dir(root)
        .output()
    else {
        return current();
    };
    anyhow::ensure!(out.status.success(), "git-cliff: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).context("git-cliff output")?;
    let text = unreleased_only(&text, &released_items(&file));
    Ok(vec![Out::block(FILE, "unreleased", section(&text))])
}

/// Every list item of the released sections, outside the Unreleased block, with its wrapped
/// lines joined. Between `make changelog` and its tag, git-cliff still counts the written
/// release as unreleased.
fn released_items(text: &str) -> HashSet<String> {
    let outside = match text.split_once(BEGIN) {
        Some((before, rest)) => [before, rest.split_once(END).map_or("", |(_, a)| a)],
        None => [text, ""],
    };
    items(&outside.join("\n")).into_iter().collect()
}

/// The list items of `text`, each with its indented continuation lines joined by a space.
fn items(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        if let Some(item) = line.strip_prefix("- ") {
            out.push(item.to_owned());
            open = true;
        } else if let (true, Some(more), Some(last)) =
            (open, line.strip_prefix("  "), out.last_mut())
        {
            last.push(' ');
            last.push_str(more.trim());
        } else {
            open = false;
        }
    }
    out
}

/// git-cliff's section without the items a release already lists, and without a heading
/// left with no item. Nothing left means nothing is unreleased.
fn unreleased_only(text: &str, released: &HashSet<String>) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        match lines.last_mut() {
            Some(last) if line.starts_with("  ") && last.starts_with("- ") => {
                last.push(' ');
                last.push_str(line.trim());
            }
            _ => lines.push(line.to_owned()),
        }
    }
    lines.retain(|l| l.strip_prefix("- ").is_none_or(|i| !released.contains(i)));
    let has_item = |from: usize| {
        lines[from + 1..]
            .iter()
            .take_while(|l| !l.starts_with("### ") && !l.starts_with("## "))
            .any(|l| l.starts_with("- "))
    };
    if !lines.iter().any(|l| l.starts_with("- ")) {
        return String::new();
    }
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("### ") && !has_item(i) {
            continue;
        }
        if line.is_empty() && out.ends_with("\n\n") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// git-cliff's section with its heading kept and each list item wrapped with a hanging
/// indent, which Markdown reads as the same item.
fn section(text: &str) -> String {
    let mut out = String::new();
    for line in text.trim().lines() {
        match line.strip_prefix("- ") {
            Some(item) => {
                let opts = textwrap::Options::new(WIDTH)
                    .initial_indent("- ")
                    .subsequent_indent("  ")
                    .break_words(false)
                    .word_splitter(textwrap::WordSplitter::NoHyphenation);
                out.push_str(&textwrap::fill(item, &opts));
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_item_wraps_under_its_dash_and_headings_stay() {
        let long = "web: ".to_owned() + &"word ".repeat(30);
        let s = section(&format!("## [Unreleased]\n\n### Added\n\n- {long}\n- short\n"));
        assert!(s.starts_with("## [Unreleased]\n\n### Added\n\n- web: word"), "{s}");
        assert!(s.lines().all(|l| l.chars().count() <= WIDTH), "{s}");
        assert!(s.lines().filter(|l| l.starts_with("  word")).count() >= 1, "{s}");
        assert!(s.ends_with("- short\n"), "{s}");
    }

    #[test]
    fn a_written_release_leaves_only_new_items_unreleased() {
        let file = format!(
            "# C\n\n## [0.1.0] - d\n\n### Added\n\n- a long\n  item\n\n### Fixed\n\n- b\n\n\
             {BEGIN}- x\n{END}\n"
        );
        let released = released_items(&file);
        assert!(!released.contains("x"), "the old block is not a release");
        let cliff = "## [Unreleased]\n\n### Added\n\n- a long\n  item\n\n### Fixed\n\n- b\n";
        assert_eq!(unreleased_only(cliff, &released), "");
        let cliff = "## [Unreleased]\n\n### Added\n\n- a long item\n- new\n\n### Fixed\n\n- b\n";
        let left = unreleased_only(cliff, &released);
        assert_eq!(left, "## [Unreleased]\n\n### Added\n\n- new\n\n");
        let cliff = "## [Unreleased]\n\n### Added\n\n- a long item\n\n### Fixed\n\n- c\n";
        let left = unreleased_only(cliff, &released);
        assert_eq!(left, "## [Unreleased]\n\n### Fixed\n\n- c\n");
    }

    #[test]
    fn a_hyphenated_word_or_flag_never_splits_across_lines() {
        let item = "export: ".to_owned() + &"word ".repeat(12) + "--max-pages and well-known";
        let s = section(&format!("- {item}\n"));
        assert!(s.lines().all(|l| !l.trim_end().ends_with('-')), "{s}");
        assert!(s.contains("--max-pages") && s.contains("well-known"), "{s}");
    }
}
