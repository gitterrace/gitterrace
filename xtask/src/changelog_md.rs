//! The `[Unreleased]` section of `CHANGELOG.md`: what changed since the last release, written
//! by git-cliff from the commits with the release's own `cliff.toml`, so the changelog page has
//! content between releases and the release step only names the version. A released section is
//! never touched here: `make changelog` writes it at the tag, and the changelog lint holds it.
//!
//! The section is as of the last `make docs-gen`. Every commit moves it, the one that writes it
//! included, so `check` keeps the text in the file instead of asking for a regenerate on every
//! commit; the changelog lint leaves Unreleased out for the same reason.

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
        let body = text
            .split_once(BEGIN)
            .and_then(|(_, r)| r.split_once(END))
            .map_or("", |(b, _)| b);
        Ok(vec![Out::block(FILE, "unreleased", body)])
    };
    if !refresh {
        return current();
    }
    let Ok(out) = Command::new("git-cliff")
        .args(["--unreleased", "--strip", "all"])
        .current_dir(root)
        .output()
    else {
        return current();
    };
    anyhow::ensure!(
        out.status.success(),
        "git-cliff: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).context("git-cliff output")?;
    Ok(vec![Out::block(FILE, "unreleased", section(&text))])
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
        let s = section(&format!(
            "## [Unreleased]\n\n### Added\n\n- {long}\n- short\n"
        ));
        assert!(
            s.starts_with("## [Unreleased]\n\n### Added\n\n- web: word"),
            "{s}"
        );
        assert!(s.lines().all(|l| l.chars().count() <= WIDTH), "{s}");
        assert!(
            s.lines().filter(|l| l.starts_with("  word")).count() >= 1,
            "{s}"
        );
        assert!(s.ends_with("- short\n"), "{s}");
    }

    #[test]
    fn a_hyphenated_word_or_flag_never_splits_across_lines() {
        let item = "export: ".to_owned() + &"word ".repeat(12) + "--max-pages and well-known";
        let s = section(&format!("- {item}\n"));
        assert!(s.lines().all(|l| !l.trim_end().ends_with('-')), "{s}");
        assert!(s.contains("--max-pages") && s.contains("well-known"), "{s}");
    }
}
