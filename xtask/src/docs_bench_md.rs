//! Generated blocks of `docs/benchmarks.md`.

use std::path::Path;

use anyhow::{Context as _, Result};

/// A benchmark block from `scripts/bench/bench-report.py`: `--corpus` lists the repos with their
/// source links from `tests/bench/corpus.toml`, `--print` renders the newest result file.
pub(crate) fn bench_report(root: &Path, flag: &str) -> Result<String> {
    let out = std::process::Command::new("python3")
        .args(["scripts/bench/bench-report.py", flag])
        .current_dir(root)
        .output()
        .context("running scripts/bench/bench-report.py")?;
    anyhow::ensure!(
        out.status.success(),
        "bench-report.py: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).context("bench-report.py output")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    /// Every `make <target>` the contributor and benchmark pages name is a target of the
    /// Makefile, so a renamed target fails here instead of leaving a command that does nothing.
    #[test]
    fn every_make_target_the_docs_name_exists() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let makefile = std::fs::read_to_string(root.join("Makefile")).expect("Makefile");
        let targets: BTreeSet<&str> = makefile
            .lines()
            .filter(|l| !l.starts_with(['\t', ' ', '#', '.']))
            .filter_map(|l| l.split_once(':'))
            .flat_map(|(names, _)| names.split_whitespace())
            .collect();
        let mut bad = Vec::new();
        for page in ["CONTRIBUTING.md", "docs/benchmarks.md"] {
            let text = std::fs::read_to_string(root.join(page)).expect("page");
            for span in text.split('`').skip(1).step_by(2) {
                let Some(rest) = span.strip_prefix("make ") else { continue };
                let name = rest.split_whitespace().next().unwrap_or_default();
                if !name.contains('=') && !targets.contains(name) {
                    bad.push(format!("{page}: make {name}"));
                }
            }
        }
        assert!(bad.is_empty(), "docs name make targets the Makefile lacks: {bad:?}");
    }
}
