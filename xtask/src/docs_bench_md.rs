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
