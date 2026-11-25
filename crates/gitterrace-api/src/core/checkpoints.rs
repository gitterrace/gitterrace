//! Checkpoints of uncommitted work and viewed marks. The reads are served on both transports.
//! The writes are the web's alone: this module offers them as plain methods and the API router
//! and the MCP server never call them.

use gitterrace_types::api::{CheckpointItem, CheckpointsResponse, ViewedItem, ViewedResponse};

use crate::core::ApiState;
use crate::error::ApiError;

/// Longest label a checkpoint keeps, in characters. Not configurable: it bounds one stored
/// entry, and a label is a short name, not a note.
const MAX_LABEL: usize = 200;

/// How many bytes of text a snapshot gathers for each byte of cap. Text packs several to one, so
/// this keeps what the cap would, and bounds memory while the files are read. A file past it
/// keeps its name and hash.
const UNPACKED_PER_PACKED: u64 = 8;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl ApiState {
    /// These are this machine's, so a committed-only (public) handle refuses them.
    pub(super) fn only_here(&self, what: &'static str) -> Result<(), ApiError> {
        if self.repo.is_committed_only() {
            return Err(ApiError::Forbidden(what));
        }
        Ok(())
    }

    fn cap_bytes(&self) -> u64 {
        (self.settings().review.checkpoint_mb as u64) << 20
    }

    /// Whether checkpoints can be kept: not switched off and no newer build owns the cache.
    fn persists(&self) -> bool {
        self.cap_bytes() > 0 && self.index.newer_owner().is_none() && self.index.dir().is_dir()
    }

    /// The checkpoints, newest first.
    pub fn checkpoints(&self) -> Result<CheckpointsResponse, ApiError> {
        self.only_here("checkpoints")?;
        let persisted = self.persists();
        let list = if persisted {
            self.index.checkpoint_list()?
        } else {
            gitterrace_index::CheckpointList::default()
        };
        Ok(CheckpointsResponse {
            bytes_used: list.bytes_used,
            cap_bytes: self.cap_bytes(),
            checkpoints: list
                .items
                .into_iter()
                .map(|c| CheckpointItem {
                    auto: c.auto,
                    bytes: c.bytes,
                    created: c.created,
                    files: c.files,
                    files_omitted: c.files_omitted,
                    head: c.head,
                    id: c.id,
                    label: c.label,
                })
                .collect(),
            persisted,
        })
    }

    /// Takes a checkpoint of the working tree now and returns its id and whether it is new
    /// (the same state again returns the old id). Web only: the API and MCP never call it.
    pub fn checkpoint_create(&self, label: &str, auto: bool) -> Result<(String, bool), ApiError> {
        self.only_here("checkpoints")?;
        if !self.persists() {
            return Err(ApiError::BadRequest(
                "checkpoints are off: review.checkpoint_mb is 0 or the cache folder cannot be \
                 written"
                    .to_owned(),
            ));
        }
        let copy = (self.settings().review.checkpoint_file_kb as u64) << 10;
        let snap = gitterrace_git::worktree_snapshot(
            &self.repo,
            copy,
            self.cap_bytes().saturating_mul(UNPACKED_PER_PACKED),
        )?;
        let head = self.repo.resolve("HEAD").unwrap_or_default();
        let label: String = label.chars().take(MAX_LABEL).collect();
        Ok(self.index.checkpoint_add(&snap, &head, auto, &label, self.cap_bytes(), now())?)
    }

    /// Whether Mark all viewed takes a checkpoint first (`review.has_auto_checkpoint`).
    #[must_use]
    pub fn has_auto_checkpoint(&self) -> bool {
        self.settings().review.has_auto_checkpoint
    }

    /// The viewed marks, oldest first.
    pub fn viewed(&self) -> Result<ViewedResponse, ApiError> {
        self.only_here("viewed marks")?;
        let viewed = self
            .index
            .viewed_list()?
            .into_iter()
            .map(|v| ViewedItem { at: v.at, path: v.path, patch: v.patch })
            .collect();
        Ok(ViewedResponse { viewed })
    }

    /// Marks each `(path, patch)` viewed, or clears the marks of those paths when `on` is false.
    /// Web only.
    pub fn viewed_set(&self, items: &[(String, String)], on: bool) -> Result<(), ApiError> {
        self.only_here("viewed marks")?;
        if self.index.newer_owner().is_some() {
            return Err(ApiError::BadRequest("the cache belongs to a newer gitterrace".to_owned()));
        }
        Ok(self.index.viewed_set(items, on, now())?)
    }
}
