//! The raw tree diff between two trees.

use gix::object::tree::diff::ChangeDetached;

use crate::Result;
use crate::error::git;

/// Blob buffers a worker's diff cache keeps between commits; see `changes_with`.
const KEEP_BLOB_BUFFERS: usize = 64;

/// The tree diff from `old` (`None` for the empty tree) to `new`, renames paired by `rewrites`.
pub(crate) fn tree_changes(
    repo: &gix::Repository,
    old: Option<&gix::Tree<'_>>,
    new_tree: &gix::Tree<'_>,
    cache: &mut gix::diff::blob::Platform,
    rewrites: gix::diff::Rewrites,
) -> Result<Vec<ChangeDetached>> {
    Ok(tree_changes_capped(repo, old, new_tree, cache, rewrites, usize::MAX)?.0)
}

/// [`tree_changes`] that stops collecting after `limit` changes. The flag says it stopped, so
/// more changes may exist than the vector holds.
pub(crate) fn tree_changes_capped(
    repo: &gix::Repository,
    old: Option<&gix::Tree<'_>>,
    new_tree: &gix::Tree<'_>,
    cache: &mut gix::diff::blob::Platform,
    rewrites: gix::diff::Rewrites,
    limit: usize,
) -> Result<(Vec<ChangeDetached>, bool)> {
    let empty = repo.empty_tree();
    let old = old.unwrap_or(&empty);
    let opts = gix::diff::Options::default().with_rewrites(Some(rewrites));
    let mut raw = Vec::new();
    let mut cut = false;
    let diffed = gix::diff::tree_with_rewrites(
        gix::objs::TreeRefIter::from_bytes(&old.data, old.id.kind()),
        gix::objs::TreeRefIter::from_bytes(&new_tree.data, new_tree.id.kind()),
        cache,
        &mut gix::diff::tree::State::default(),
        &repo.objects,
        |change| {
            if raw.len() >= limit {
                cut = true;
                return Ok(std::ops::ControlFlow::Break(()));
            }
            raw.push(change.into_owned());
            Ok(std::ops::ControlFlow::Continue(()))
        },
        opts.into(),
    );
    // Keeping the buffers saves an allocation per blob on the next commit, but after a mass
    // rename the free list holds one buffer per compared blob (thousands, megabytes each) and
    // only halves per commit. Ten workers holding that cost hundreds of MB of peak memory.
    if cache.clear_resource_cache_keep_allocation() > KEEP_BLOB_BUFFERS {
        cache.clear_resource_cache();
    }
    // A stop we asked for may surface as an error from the walk: the changes so far stand.
    if !cut {
        diffed.map_err(git)?;
    }
    Ok((raw, cut))
}
