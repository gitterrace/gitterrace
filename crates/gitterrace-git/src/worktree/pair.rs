//! Exact renames in the working tree: a deleted file whose bytes reappear under an added path,
//! as `git status` shows a `git mv`. Inexact renames stay delete plus add: similarity search
//! over every pair costs too much on a watcher tick.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::Path;

use gix::ObjectId;

#[derive(Default)]
pub(super) struct Moves {
    pub(super) to: HashMap<String, String>,
    /// Deleted paths that became a rename, so they are not listed again as deletions.
    pub(super) sources: BTreeSet<String>,
}

/// Pairs on the blob id: HEAD's tree already names each deleted file's id, and an added file
/// is hashed as it is read, one at a time, so only one file's bytes are held at once. Empty
/// files never pair (every empty file has the same id). Sources pair in path order.
pub(super) fn exact_moves(
    r: &gix::Repository,
    head: Option<&gix::Tree<'_>>,
    root: Option<&Path>,
    paths: &BTreeSet<String>,
    big: u64,
) -> Moves {
    let (Some(head), Some(root)) = (head, root) else {
        return Moves::default();
    };
    let mut gone: HashMap<ObjectId, VecDeque<&str>> = HashMap::new();
    let mut sizes = BTreeSet::new();
    let mut added = Vec::new();
    for path in paths {
        let on_disk = std::fs::symlink_metadata(root.join(path));
        let in_head = crate::treepath::lookup(head, path).ok().flatten();
        match (in_head, on_disk) {
            (Some(e), Err(err)) if err.kind() == std::io::ErrorKind::NotFound => {
                let id = e.object_id();
                let size = r.find_header(id).map_or(0, |h| h.size());
                if e.mode().is_blob() && size > 0 {
                    gone.entry(id).or_default().push_back(path);
                    sizes.insert(size);
                }
            }
            (None, Ok(m)) if m.is_file() && m.len() > 0 => added.push((path, m.len())),
            _ => {}
        }
    }
    let mut out = Moves::default();
    if gone.is_empty() {
        return out;
    }
    for (path, len) in added {
        // Only a file of a deleted file's exact size can be its copy: no read for the rest.
        if !sizes.contains(&len) || len > big {
            continue;
        }
        // The view's own reader: it refuses a path behind a symlinked directory.
        let crate::worktree::Read::Data(bytes) = &crate::worktree::read_worktree(root, path, big)
        else {
            continue;
        };
        let Ok(id) = gix::objs::compute_hash(r.object_hash(), gix::objs::Kind::Blob, bytes) else {
            continue;
        };
        if let Some(from) = gone.get_mut(&id).and_then(VecDeque::pop_front) {
            out.to.insert(path.clone(), from.to_owned());
            out.sources.insert(from.to_owned());
        }
    }
    out
}
