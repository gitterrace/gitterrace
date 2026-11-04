//! The change list of recent commits, kept for one export run so a commit page and its folded
//! files share one tree diff. A server never holds one: its memory is the caches' budget.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use gix::object::tree::diff::ChangeDetached;

use crate::{Error, Result};

/// Commits whose change lists are kept. An export fetches a batch of commit pages at once (two
/// per core) and their folded files right after, so this covers a batch on a large machine.
/// An entry is a list of changed paths and ids, not blobs: kilobytes, rarely megabytes.
const KEEP_COMMITS: usize = 64;

type Changes = Arc<Vec<ChangeDetached>>;
/// Filled once; a thread asking while another computes it waits instead of diffing again.
type Slot = Arc<OnceLock<std::result::Result<Changes, String>>>;

/// The newest commits' tree diffs, oldest first.
///
/// A change list depends only on the commit, so a hit is the same answer, and dropping an
/// entry costs only a recomputation. The caller that
/// runs the export owns it ([`crate::Repo::use_change_memo`]); a handle only points at it, so
/// dropping the owner frees every list whatever handles outlive the run.
#[derive(Default)]
pub struct ChangeMemo {
    slots: Mutex<VecDeque<(gix::ObjectId, Slot)>>,
}

impl ChangeMemo {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The change list of `commit`, computed by `diff` only on a miss.
    pub(crate) fn get(
        &self,
        commit: gix::ObjectId,
        diff: impl FnOnce() -> Result<Vec<ChangeDetached>>,
    ) -> Result<Changes> {
        let slot = self.slot(commit);
        let got = slot.get_or_init(|| diff().map(Arc::new).map_err(|e| e.to_string()));
        match got {
            Ok(changes) => Ok(Arc::clone(changes)),
            Err(e) => {
                // A failure is not kept: the next request tries again.
                self.forget(commit, &slot);
                Err(Error::Git(e.clone()))
            }
        }
    }

    fn slot(&self, commit: gix::ObjectId) -> Slot {
        let Ok(mut slots) = self.slots.lock() else {
            return Slot::default();
        };
        let hit = slots.iter().position(|(id, _)| *id == commit);
        if let Some(hit) = hit.and_then(|i| slots.remove(i)) {
            let slot = Arc::clone(&hit.1);
            slots.push_back(hit);
            return slot;
        }
        if slots.len() >= KEEP_COMMITS {
            slots.pop_front();
        }
        let slot = Slot::default();
        slots.push_back((commit, Arc::clone(&slot)));
        slot
    }

    fn forget(&self, commit: gix::ObjectId, slot: &Slot) {
        if let Ok(mut slots) = self.slots.lock() {
            slots.retain(|(id, s)| !(*id == commit && Arc::ptr_eq(s, slot)));
        }
    }
}
