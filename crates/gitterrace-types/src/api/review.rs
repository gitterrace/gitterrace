//! Checkpoints of uncommitted work and viewed marks: the answers of `/checkpoints` and
//! `/viewed`. Both are this machine's own data, so a public server refuses them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::git::{AnchorBasis, AnchorStatus};

/// One checkpoint of the uncommitted work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CheckpointItem {
    /// Taken by Mark all viewed (`review.has_auto_checkpoint`), not by the Checkpoint button.
    pub auto: bool,
    /// Bytes of file copies the checkpoint still holds.
    pub bytes: u64,
    /// Seconds since the Unix epoch.
    pub created: i64,
    pub files: usize,
    /// The scan of the working tree stopped at `limits.worktree_files`, so the checkpoint may
    /// not hold every changed file. Raise the limit and take another checkpoint.
    pub files_omitted: bool,
    /// Changed text files saved without a copy because the checkpoint reached the restore
    /// budget (256 MiB unpacked). They keep path and hash, so a diff says they changed.
    pub copies_omitted: usize,
    /// The commit HEAD was at.
    pub head: String,
    /// `cp-` and 12 hex digits of the hash of the state: the same state is the same id.
    pub id: String,
    pub label: String,
}

/// `GET /checkpoints`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CheckpointsResponse {
    pub bytes_used: u64,
    /// Older checkpoints discarded whole to keep the log's headers within the reader's
    /// 64 MiB bound, in all. Not rebuildable: a nonzero count means checkpoints are gone.
    pub dropped: usize,
    /// `review.checkpoint_mb` in bytes. 0 means checkpoints are off.
    pub cap_bytes: u64,
    /// Newest first.
    pub checkpoints: Vec<CheckpointItem>,
    /// False when checkpoints cannot be kept: there is no writable cache folder, a newer build
    /// owns it, or the cap is 0.
    pub persisted: bool,
}

/// One file marked as viewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ViewedItem {
    /// Seconds since the Unix epoch.
    pub at: i64,
    /// The path as at the working tree tip.
    pub path: String,
    /// 16 hex digits of the hash of the file's diff. The mark holds only while it matches.
    pub patch: String,
}

/// `GET /viewed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ViewedResponse {
    pub viewed: Vec<ViewedItem>,
}

/// A range of lines, 1-based and inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingLines {
    pub end: u32,
    pub start: u32,
}

/// Where the code a finding points at is now, worked out on every read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FindingAnchor {
    /// `head`, or `worktree` when the uncommitted changes were taken into account.
    pub checked_against: AnchorBasis,
    /// When the status is `untouched` and the lines moved, or `lines edited`: where they are
    /// now. For `lines edited` it is the span the diff maps the old lines to, not a guess.
    pub now_lines: Option<FindingLines>,
    /// The path now, after following renames. Null when the file is gone.
    pub now_path: Option<String>,
    /// What happened to the code: `untouched`, `lines changed`, `lines edited`,
    /// `file changed` or `file gone`.
    /// It never says whether the finding was dealt with.
    pub status: AnchorStatus,
}

/// One entry of the review basket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Finding {
    /// Seconds since the Unix epoch.
    pub added: i64,
    pub anchor: FindingAnchor,
    /// `f-` and 8 hex digits of the hash of the entry.
    pub id: String,
    /// What was selected: `line`, `file`, `commit`, `hunk`, `blame`, `search-hit`, `insight` or
    /// `page`.
    pub kind: String,
    pub lines: Option<FindingLines>,
    /// Why it matters, at most 2000 bytes. Plain text: no markup, escaped wherever it is shown.
    pub note: String,
    pub path: Option<String>,
    pub query: Option<String>,
    /// The revision the page was showing.
    pub rev: String,
    /// The full commit id the finding is pinned to.
    pub sha: String,
}

/// `GET /findings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FindingsResponse {
    /// `review.findings_entries`: the basket drops its oldest entry past this.
    pub cap_entries: usize,
    pub count: usize,
    /// Oldest first.
    pub findings: Vec<Finding>,
    /// False when the basket cannot be kept: there is no writable cache folder or a newer build
    /// owns it.
    pub persisted: bool,
}

/// A finding as the page sends it: everything but what the server fills in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewFinding {
    pub kind: String,
    pub lines: Option<FindingLines>,
    pub note: String,
    pub path: Option<String>,
    pub query: Option<String>,
    pub rev: String,
    pub sha: Option<String>,
}
