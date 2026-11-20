//! Generated files are tracked and marked `linguist-generated` in `.gitattributes`, so a diff
//! view folds them and a reader knows not to edit them. A generator's whole-file output under
//! `docs/` with no mark, a mark on a `docs/` file nothing generates, or a bench result pattern
//! gone from the file is a finding of `check`, like a stale file.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::out::Out;

const ATTRIBUTES: &str = ".gitattributes";
const FLAG: &str = "linguist-generated";
/// The bench charts and per-run pages are drawn from the run JSON and not tracked, so no
/// generator lists them: the attributes name them as patterns, and these must stay.
const BENCH: [&str; 3] = [
    "tests/bench/results/*.json",
    "tests/bench/results/**/*.svg",
    "tests/bench/results/**/README.md",
];

/// The paths and patterns `.gitattributes` marks generated.
fn marked(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            let pattern = words.next()?;
            words.any(|w| w == FLAG).then_some(pattern)
        })
        .collect()
}

/// Whether `out` is a document a reader opens: a whole `.md` or `.json` file under `docs/`.
fn is_doc(out: &Out) -> Option<&str> {
    let Out::File { path, .. } = out else {
        return None;
    };
    let under = path.starts_with("docs/") && !path.starts_with("docs/brand/");
    let ext = Path::new(path).extension().and_then(|e| e.to_str());
    (under && matches!(ext, Some("md" | "json"))).then_some(path.as_str())
}

/// What is wrong with the marks in `attributes`, given the generators' `outs`.
fn problems(attributes: &str, outs: &[Out]) -> Vec<String> {
    let marks = marked(attributes);
    let docs: Vec<&str> = outs.iter().filter_map(is_doc).collect();
    let mut bad = Vec::new();
    for doc in &docs {
        if !marks.contains(doc) {
            bad.push(format!(
                "{doc} is generated but {ATTRIBUTES} does not mark it {FLAG}"
            ));
        }
    }
    for mark in marks
        .iter()
        .filter(|m| m.starts_with("docs/") && !m.contains('*'))
    {
        if Path::new(mark).extension().is_some_and(|e| e == "md") && !docs.contains(mark) {
            bad.push(format!(
                "{ATTRIBUTES} marks {mark} {FLAG}, but no generator writes it"
            ));
        }
    }
    for pattern in BENCH {
        if !marks.contains(&pattern) {
            bad.push(format!("{ATTRIBUTES} lacks {FLAG} for {pattern}"));
        }
    }
    bad
}

/// The marks that are missing or stale, as findings.
pub fn check(root: &Path, outs: &[Out]) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(root.join(ATTRIBUTES)).context("reading .gitattributes")?;
    Ok(problems(&text, outs))
}

#[cfg(test)]
mod tests;
