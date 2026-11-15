//! Generated files in code search: tagged and listed after the hand-written ones.

use crate::{app::App, error::Result, search::FileHits};

/// Marks the generated files among `files` (`gitterrace_git::generated_paths`) and moves
/// them after the others, keeping path order within each group, so lockfile hits never bury
/// the hand-written ones.
pub(super) fn generated_last(
    a: &App,
    rev: &str,
    mut files: Vec<FileHits>,
) -> Result<Vec<FileHits>> {
    let generated =
        gitterrace_git::generated_paths(&a.repo, rev, files.iter().map(|f| f.path.as_str()))?;
    for f in &mut files {
        f.generated = generated.contains(&f.path);
    }
    files.sort_by_key(|f| f.generated);
    Ok(files)
}
