//! The version stamp's dirty check, shared by build.rs and its tests.

use std::{path::Path, process::Command};

/// Pathspec for the dirty check. `:(top)` makes it cover the whole worktree, because
/// cargo runs build scripts in the crate directory. Result files are output of the
/// bench, not input to the build, so they never make it dirty.
pub const DIRTY_SPEC: [&str; 2] = [":(top)", ":(top,exclude)bench/results"];

/// True when `git status` run in `dir` reports a change that counts for the stamp.
pub fn is_dirty(dir: &Path) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["status", "--porcelain", "--", DIRTY_SPEC[0], DIRTY_SPEC[1]])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !o.stdout.is_empty())
}
