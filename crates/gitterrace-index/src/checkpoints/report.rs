//! What a checkpoint write reports and the budgets it works within.

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
pub(super) const fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The budgets one checkpoint write works within, as arguments so a test can use small ones.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budgets {
    /// The caller's `cap_bytes`: most bytes of all checkpoints, oldest dropped first.
    pub cap: u64,
    /// Most bytes of headers one log holds.
    pub headers: u64,
    /// Most unpacked bytes of copies one checkpoint keeps.
    pub restore: u64,
}

impl Budgets {
    /// The reader's budgets with the caller's cap.
    pub(crate) const fn with_cap(cap: u64) -> Self {
        Self {
            cap,
            headers: crate::checkpoints::MAX_HEADERS,
            restore: crate::checkpoints::MAX_RESTORE,
        }
    }
}

/// What recording a checkpoint did beyond storing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddReport {
    pub id: String,
    /// False when the newest checkpoint already held this state.
    pub is_new: bool,
    /// Older checkpoints discarded whole by this write to keep the headers readable.
    pub dropped: usize,
    /// Text files of this checkpoint whose copy was not kept (restore budget).
    pub copies_omitted: usize,
}
