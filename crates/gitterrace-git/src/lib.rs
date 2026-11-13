//! Read-only access to a git repository. Every read is pinned to a commit;
//! the working tree is never touched.

mod atomic;
mod diff;
mod divergence;
mod error;
mod fnv;
mod fs_events;
mod gitdir;
mod graph;
mod ident;
mod ignore;
mod nudge;
mod repo;
mod repo_id;
mod roots;
mod text;
mod time;
mod treepath;
mod types;
mod url;
mod walk;
mod watch;
mod worktree;

pub use atomic::{
    open_append_no_follow, open_read_no_follow, read_no_follow_bounded, refused, write_atomic,
};
pub use error::{Error, Result};
pub use fnv::{fnv1a32, fnv1a64};

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
/// Not configurable itself: `limits.text_kb` sets the display cap per handle; this constant is
/// the default and the index's fixed bound, so stored numbers never depend on a setting.
pub const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
pub use gitdir::{BranchStatus, HeadState, InProgress, LinkedWorktree, Stash, TagAnnotation};
pub use ignore::IgnoreSet;
pub use nudge::{Nudge, NudgeConfig, NudgeError, Nudger, Skip};
pub use repo::{ChangeMemo, REJECTED_REV, Repo};
pub use repo_id::resolve_permalink;
pub use roots::Roots;
pub use text::{CommitText, Mailmap};
pub use time::{DAY, FIRST_SECOND, LAST_SECOND, civil_from_days, days_from_civil, iso_utc, ymd};
pub use types::{
    BlameLine, Blob, ChangeKind, CodeCounts, CommitDetail, CommitSummary, FileDiff, Hunk, HunkLine,
    LineKind, RawBlob, RefInfo, RefKind, Signature, TreeEntry, TreeEntryKind,
};
pub use url::{percent_decode, percent_encode_path, percent_encode_segment};
pub use walk::{
    BlobId, BlobReader, CommitScan, FileRef, LastCommit, canonical_order, files, last_commits,
    last_commits_among, scan_commits, tree_id,
};
pub use watch::{Parts, Watcher};
pub use worktree::{
    WORKTREE_DIFF_BYTES, WORKTREE_FILES, is_dirty, worktree_bytes, worktree_diff,
    worktree_file_diff, worktree_signature,
};
