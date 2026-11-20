//! Stamps `GITTERRACE_VERSION`: `v0.1.0` on a release tag, else `v0.1.0-<sha>` with
//! `-dirty` for uncommitted changes outside tests/bench/results.
//! Outside a git checkout it is just `v0.1.0`.

use std::{
    io::Write as _,
    process::{Command, Output},
};

mod stamp;

fn git(args: &[&str]) -> Option<Output> {
    Command::new("git").args(args).output().ok().filter(|o| o.status.success())
}

fn version(base: &str) -> String {
    if git(&["describe", "--exact-match", "--tags", "HEAD"]).is_some() {
        return format!("v{base}");
    }
    let sha = git(&["rev-parse", "--short", "HEAD"])
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty());
    let dirty = stamp::is_dirty(std::path::Path::new("."));
    match (sha, dirty) {
        (Some(s), true) => format!("v{base}-{s}-dirty"),
        (Some(s), false) => format!("v{base}-{s}"),
        _ => format!("v{base}"),
    }
}

fn main() {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let version = match std::env::var("GITTERRACE_VERSION") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => version(&base),
    };
    let mut out = std::io::stdout().lock();
    for line in [
        format!("cargo:rustc-env=GITTERRACE_VERSION={version}"),
        "cargo:rerun-if-env-changed=GITTERRACE_VERSION".to_owned(),
        "cargo:rerun-if-changed=../../.git/HEAD".to_owned(),
        "cargo:rerun-if-changed=../../.git/refs/".to_owned(),
        "cargo:rerun-if-changed=../../.git/packed-refs".to_owned(),
        "cargo:rerun-if-changed=../../.git/index".to_owned(),
        "cargo:rerun-if-changed=stamp.rs".to_owned(),
    ] {
        let _ = writeln!(out, "{line}");
    }
}
