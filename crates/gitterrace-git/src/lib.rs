//! Read-only access to a git repository. Every read is pinned to a commit;
//! the working tree is never touched.

mod atomic;
mod diff;
mod error;
mod gitdir;
mod ident;
mod ignore;
mod repo;
mod repo_id;
mod time;
mod treepath;
mod types;
mod url;
mod walk;
mod watch;
mod worktree;

pub use atomic::{open_append_no_follow, read_no_follow, write_atomic};
pub use error::{Error, Result};

/// How many hex digits of a commit id any surface shows. One constant, so a short id looks the
/// same in the web pages, the action payloads and the CLI.
const SHORT_ID_LEN: usize = 8;

/// The abbreviated form of a commit id (or the whole string when it is already shorter).
#[must_use]
pub fn short_id(id: &str) -> &str {
    id.get(..SHORT_ID_LEN).unwrap_or(id)
}

/// Largest blob read as text anywhere: blob views, blame, line counts and diffs.
///
/// Checked against the object header before reading, so a huge blob never reaches memory; larger
/// blobs are treated like binary ones.
pub const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
pub use gitdir::{BranchStatus, HeadState, InProgress, LinkedWorktree, Stash, TagAnnotation};
pub use ignore::IgnoreSet;
pub use repo::Repo;
pub use repo_id::resolve_permalink;
pub use time::{civil_from_days, days_from_civil};
pub use types::{
    BlameLine, Blob, ChangeKind, CodeCounts, CommitDetail, CommitSummary, FileDiff, Hunk, HunkLine,
    LineKind, RawBlob, RefInfo, RefKind, Signature, TreeEntry, TreeEntryKind,
};
pub use url::{percent_decode, percent_encode_path, percent_encode_segment};
pub use walk::{
    BlobId, BlobReader, FileRef, LastCommit, canonical_order, files, last_commits,
    last_commits_among, scan_commits, tree_id,
};
#[cfg(test)]
pub use watch::Cost;
pub use watch::{Parts, Watcher};
pub use worktree::{is_dirty, worktree_diff, worktree_signature};
