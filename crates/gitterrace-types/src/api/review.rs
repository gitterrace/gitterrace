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
    /// Only when the status is `untouched` and the lines moved: where the same lines are now.
    pub now_lines: Option<FindingLines>,
    /// The path now, after following renames. Null when the file is gone.
    pub now_path: Option<String>,
    /// What happened to the code: `untouched`, `lines changed`, `file changed` or `file gone`.
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
