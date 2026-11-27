//! Checkpoints of uncommitted work and viewed marks. The reads are served on both transports.
//! The writes are the web's alone: this module offers them as plain methods and the API router
//! and the MCP server never call them.

use gitterrace_types::api::{CheckpointItem, CheckpointsResponse, ViewedItem, ViewedResponse};

use crate::core::ApiState;
use crate::error::ApiError;

/// How many bytes of text a snapshot gathers for each byte of cap. Text packs several to one, so
/// this keeps what the cap would, and bounds memory while the files are read. A file past it
/// keeps its name and hash.
const UNPACKED_PER_PACKED: u64 = 8;

/// The shortest and longest `since` duration, in seconds: a minute to a week. Not configurable:
/// a request bound on the syntax, not a resource cap. An older base is named by its id.
const SINCE_MIN: i64 = 60;
const SINCE_MAX: i64 = 7 * 24 * 3600;

/// Seconds in a duration such as `90s`, `15m`, `2h` or `1d`, `None` when it is not one.
fn parse_duration(text: &str) -> Option<i64> {
    let unit = match text.chars().last()? {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86400,
        _ => return None,
    };
    let digits = &text[..text.len() - 1];
    if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok().map(|n| n * unit)
}

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
            dropped: list.dropped,
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
                    copies_omitted: c.copies_omitted,
                    head: c.head,
                    id: c.id,
                    label: c.label,
                })
                .collect(),
            persisted,
        })
    }

    /// The checkpoint id `since` names: an id as it is, or a duration (`15m`, one minute to
    /// seven days) as the newest checkpoint taken at or before that long ago.
    ///
    /// # Errors
    /// A duration out of range is a bad request. When no checkpoint is that old the error is
    /// `checkpoint_not_found` and names the oldest one's age.
    pub fn resolve_since(&self, since: &str) -> Result<String, ApiError> {
        let Some(secs) = parse_duration(since) else {
            return Ok(since.to_owned());
        };
        if !(SINCE_MIN..=SINCE_MAX).contains(&secs) {
            return Err(ApiError::BadRequest(format!(
                "since {since:?} is out of range: use 1m to 7d, or a checkpoint id"
            )));
        }
        self.only_here("checkpoints")?;
        let list = if self.persists() { self.index.checkpoint_list()?.items } else { Vec::new() };
        let at = now() - secs;
        list.iter().find(|c| c.created <= at).map(|c| c.id.clone()).ok_or_else(|| {
            let oldest = list.last().map_or_else(
                || "none is kept".to_owned(),
                |c| format!("the oldest was taken {}s ago", (now() - c.created).max(0)),
            );
            ApiError::CheckpointNotFound(format!("none at or before {since} ago ({oldest})"))
        })
    }

    /// Takes a checkpoint of the working tree now and returns its id and whether it is new
    /// (the same state again returns the old id). Tests only: a caller that shows the result to
    /// a user takes [`Self::checkpoint_create_report`], so a drop or an omission is never lost.
    #[cfg(test)]
    pub fn checkpoint_create(&self, label: &str, auto: bool) -> Result<(String, bool), ApiError> {
        let r = self.checkpoint_create_report(label, auto)?;
        Ok((r.id, r.is_new))
    }

    /// Takes a checkpoint of the working tree now and reports what the write gave up: older
    /// checkpoints dropped for the header budget and copies not kept for the restore budget,
    /// for the caller to show. The same state again returns the old id.
    pub fn checkpoint_create_report(
        &self,
        label: &str,
        auto: bool,
    ) -> Result<gitterrace_index::AddReport, ApiError> {
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
        Ok(self.index.checkpoint_add_report(&snap, &head, auto, label, self.cap_bytes(), now())?)
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

    /// Marks the items whose path has no record yet, an unmark counting as a record. One
    /// critical section for the check and the write. Answers how many were marked. Web only.
    pub fn viewed_add_missing(&self, items: &[(String, String)]) -> Result<usize, ApiError> {
        self.only_here("viewed marks")?;
        if self.index.newer_owner().is_some() {
            return Err(ApiError::BadRequest("the cache belongs to a newer gitterrace".to_owned()));
        }
        Ok(self.index.viewed_add_missing(items, now())?)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_duration;

    #[test]
    fn a_duration_has_digits_and_a_unit_and_an_id_is_not_one() {
        assert_eq!(parse_duration("15m"), Some(900));
        assert_eq!(parse_duration("2h"), Some(7200));
        assert_eq!(parse_duration("1d"), Some(86400));
        assert_eq!(parse_duration("90s"), Some(90));
        for no in ["", "m", "cp-0123456789am", "-5m", "1.5h", "5x", "9999999999m"] {
            assert_eq!(parse_duration(no), None, "{no}");
        }
    }
}
