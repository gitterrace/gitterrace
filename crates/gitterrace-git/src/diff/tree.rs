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

/// The paths a bounded tree walk found.
pub(crate) struct WalkedPaths {
    /// Every changed non-tree path, a rename as its removal and its addition. At most `limit`.
    pub paths: Vec<String>,
    /// The walk stopped at the limit, so more paths may have changed.
    pub cut: bool,
    /// Entries the walk reported, trees included: the work done, which a test bounds.
    #[cfg(test)]
    pub visited: usize,
}

/// Collects paths and stops the traversal itself at `limit`, so nothing is buffered past it.
struct Capped {
    rec: gix::diff::tree::Recorder,
    limit: usize,
    out: WalkedPaths,
}

impl gix::diff::tree::Visit for Capped {
    fn pop_front_tracked_path_and_set_current(&mut self) {
        self.rec.pop_front_tracked_path_and_set_current();
    }
    fn push_back_tracked_path_component(&mut self, c: &gix::bstr::BStr) {
        self.rec.push_back_tracked_path_component(c);
    }
    fn push_path_component(&mut self, c: &gix::bstr::BStr) {
        self.rec.push_path_component(c);
    }
    fn pop_path_component(&mut self) {
        self.rec.pop_path_component();
    }
    fn visit(&mut self, change: gix::diff::tree::visit::Change) -> gix::diff::tree::visit::Action {
        #[cfg(test)]
        {
            self.out.visited += 1;
        }
        if change.entry_mode().is_tree() {
            return std::ops::ControlFlow::Continue(());
        }
        if self.out.paths.len() >= self.limit {
            self.out.cut = true;
            return std::ops::ControlFlow::Break(());
        }
        self.out.paths.push(self.rec.path_clone().to_string());
        std::ops::ControlFlow::Continue(())
    }
}

/// The paths that differ between two trees without rewrite tracking, which would buffer every
/// change before any limit applies. A rename shows as its two paths, so nothing is lost. The
/// traversal stops at `limit` paths.
pub(crate) fn tree_paths_capped(
    repo: &gix::Repository,
    old: &gix::Tree<'_>,
    new_tree: &gix::Tree<'_>,
    limit: usize,
) -> Result<WalkedPaths> {
    let mut delegate = Capped {
        rec: gix::diff::tree::Recorder::default(),
        limit,
        out: WalkedPaths {
            paths: Vec::new(),
            cut: false,
            #[cfg(test)]
            visited: 0,
        },
    };
    let walked = gix::diff::tree(
        gix::objs::TreeRefIter::from_bytes(&old.data, old.id.kind()),
        gix::objs::TreeRefIter::from_bytes(&new_tree.data, new_tree.id.kind()),
        gix::diff::tree::State::default(),
        &repo.objects,
        &mut delegate,
    );
    // A stop we asked for may surface as an error: the paths so far stand.
    if !delegate.out.cut {
        walked.map_err(git)?;
    }
    Ok(delegate.out)
}
