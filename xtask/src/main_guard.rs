//! The checkout that main is landed into never has its tracked files written by a generator.
//! Every change reaches it through the landing tool, so a generate run there leaves a dirty tree
//! that the next landing refuses. The landing tool marks that checkout with `land-tip` in its git
//! dir; a clone never has one.

use std::path::Path;

use anyhow::{Result, bail};

/// The variable that lets a generate run in the landing checkout on purpose.
pub const ALLOW: &str = "GITTERRACE_ALLOW_MAIN";

/// Refuses when `root` is the checkout the landing tool lands into, unless [`ALLOW`] is `1`.
///
/// # Errors
/// When `root/.git/land-tip` exists and the override is not set.
pub fn refuse_main(root: &Path, allowed: bool) -> Result<()> {
    if allowed || !root.join(".git").join("land-tip").is_file() {
        return Ok(());
    }
    bail!(
        "this is the checkout main is landed into, so generate writes no tracked file here: run \
         it in a clone (.wt/<lane>), or set {ALLOW}=1 to write here on purpose"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard: a generate run in the landing checkout is refused, a clone and the override pass.
    #[test]
    fn generate_refuses_the_landing_checkout_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join(".git")).expect("git dir");
        refuse_main(dir.path(), false).expect("a clone has no land-tip");
        std::fs::write(dir.path().join(".git").join("land-tip"), "0\n").expect("land-tip");
        let err = refuse_main(dir.path(), false).expect_err("the landing checkout");
        assert!(err.to_string().contains(ALLOW), "{err}");
        refuse_main(dir.path(), true).expect("the override");
    }
}
