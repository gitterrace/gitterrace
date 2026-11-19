//! The `--ref A..B` form: the compare page of a repository.

use std::path::Path;

use anyhow::Context;
use gitterrace_git::Repo;

use crate::open::{Target, show};

/// `git terrace --ref A..B` or `A...B`: the compare page of the repository at `clone`.
pub(crate) fn run_range(
    spec: &str,
    clone: &Path,
    print: bool,
    loaded: &gitterrace_config::Loaded,
    config_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let repo = Repo::discover(clone).context("opening the repository")?;
    let (base, head, merge) = parse_range(spec)?;
    for rev in [&base, &head] {
        repo.resolve(rev)
            .map_err(|_| anyhow::anyhow!("no such revision in this repository: {rev}"))?;
    }
    let target = Target {
        clone: repo.root(),
        path: gitterrace_web::compare_path(Some(&base), &head, merge),
        note: None,
    };
    show(&target, print, loaded, config_dir)
}

/// `A..B` or `A...B` as base, head and whether it is the three-dot form. An empty side is
/// `HEAD`, as in git.
pub(super) fn parse_range(spec: &str) -> anyhow::Result<(String, String, bool)> {
    let (base, head, merge) = if let Some((a, b)) = spec.split_once("...") {
        (a, b, true)
    } else if let Some((a, b)) = spec.split_once("..") {
        (a, b, false)
    } else {
        anyhow::bail!("expected A..B or A...B, got {spec}");
    };
    let side = |s: &str| {
        if s.is_empty() {
            "HEAD".to_owned()
        } else {
            s.to_owned()
        }
    };
    Ok((side(base), side(head), merge))
}
