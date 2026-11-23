//! Checkpoints of uncommitted work and viewed marks: the answers of `/checkpoints` and
//! `/viewed`. Both are this machine's own data, so a public server refuses them.

use schemars::JsonSchema;
use serde::Serialize;

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
