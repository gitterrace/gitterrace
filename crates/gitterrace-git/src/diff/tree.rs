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
    let empty = repo.empty_tree();
    let old = old.unwrap_or(&empty);
    let opts = gix::diff::Options::default().with_rewrites(Some(rewrites));
    let mut raw = Vec::new();
    let diffed = gix::diff::tree_with_rewrites(
        gix::objs::TreeRefIter::from_bytes(&old.data, old.id.kind()),
        gix::objs::TreeRefIter::from_bytes(&new_tree.data, new_tree.id.kind()),
        cache,
        &mut gix::diff::tree::State::default(),
        &repo.objects,
        |change| {
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
    diffed.map_err(git)?;
    Ok(raw)
}
