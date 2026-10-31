//! Read-only access to a git repository. Every read is pinned to a commit;
//! the working tree is never touched.

mod error;
mod repo;
mod types;

pub use error::{Error, Result};
pub use repo::Repo;
pub use types::{
    BlameLine, Blob, ChangeKind, CommitDetail, CommitSummary, FileDiff, Hunk, HunkLine, LineKind,
    RefInfo, RefKind, Signature, TreeEntry, TreeEntryKind,
};
