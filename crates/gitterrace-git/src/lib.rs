//! Read-only access to a git repository. Every read is pinned to a commit;
//! the working tree is never touched.

// Every fallible read fails the same way, so per-function `# Errors` sections add nothing.
#![allow(clippy::missing_errors_doc)]

mod diff;
mod error;
mod ident;
mod repo;
mod types;
mod walk;
mod worktree;

pub use error::{Error, Result};
pub use repo::Repo;
pub use types::{
    BlameLine, Blob, ChangeKind, CommitDetail, CommitSummary, FileDiff, Hunk, HunkLine, LineKind,
    RefInfo, RefKind, Signature, TreeEntry, TreeEntryKind,
};
pub use walk::{
    BlobId, BlobReader, FileRef, LastCommit, files, last_commits, scan_commits, tree_id,
};
pub use worktree::{is_dirty, worktree_diff};
