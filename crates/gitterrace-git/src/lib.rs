//! Read-only access to a git repository. Every read is pinned to a commit;
//! the working tree is never touched.

mod diff;
mod error;
mod gitdir;
mod ident;
mod ignore;
mod repo;
mod repo_id;
mod types;
mod walk;
mod watch;
mod worktree;

pub use error::{Error, Result};

/// Largest blob read as text anywhere: blob views, blame, line counts and diffs. Checked
/// against the object header before reading, so a huge blob never reaches memory; larger
/// blobs are treated like binary ones.
pub const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
pub use gitdir::{BranchStatus, HeadState, InProgress, LinkedWorktree, Stash, TagAnnotation};
pub use ignore::IgnoreSet;
pub use repo::Repo;
pub use repo_id::resolve_permalink;
pub use types::{
    BlameLine, Blob, ChangeKind, CodeCounts, CommitDetail, CommitSummary, FileDiff, Hunk, HunkLine,
    LineKind, RefInfo, RefKind, Signature, TreeEntry, TreeEntryKind,
};
pub use walk::{
    BlobId, BlobReader, FileRef, LastCommit, files, last_commits, scan_commits, tree_id,
};
pub use watch::{Cost, Watcher};
pub use worktree::{is_dirty, worktree_diff};
