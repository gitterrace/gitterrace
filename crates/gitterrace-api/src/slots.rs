//! Concurrency caps that a config change can lower while slots are busy. The only way to a
//! permit, so no holder can return one past a cut that is still owed.

use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;

/// A semaphore with the number of slots in force beside it, so a resize cannot drift from it.
///
/// Clones share both. Permits come out as [`Slot`]s: a cut that found slots busy is owed, and
/// each slot that comes back pays it off instead of returning, so the lower cap always lands.
#[derive(Clone)]
pub struct Slots {
    sem: Arc<Semaphore>,
    state: Arc<Mutex<SlotCount>>,
}

/// `cap` permits exist, of which `owed` are to be retired as they come back.
struct SlotCount {
    cap: usize,
    owed: usize,
}

/// Permits held out of a [`Slots`]. Dropping it returns them, minus whatever the cap still owes.
pub struct Slot {
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    slots: Slots,
}

impl Slots {
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(n)),
            state: Arc::new(Mutex::new(SlotCount { cap: n, owed: 0 })),
        }
    }

    /// Moves to `new` slots. Slots in use are not taken back at once: the cut that could not
    /// be made is owed and settles as they are released. A raise pays the debt off first.
    pub fn resize(&self, new: usize) {
        let Ok(mut st) = self.state.lock() else { return };
        let target = st.cap - st.owed;
        if new >= target {
            let paid = st.owed.min(new - target);
            st.owed -= paid;
            let added = new - target - paid;
            self.sem.add_permits(added);
            st.cap += added;
        } else {
            let want = target - new;
            let forgot = self.sem.forget_permits(want);
            st.cap -= forgot;
            st.owed += want - forgot;
        }
    }

    /// Waits for one slot.
    ///
    /// # Errors
    /// When the slots are closed.
    pub async fn take(&self) -> Result<Slot, tokio::sync::AcquireError> {
        self.take_many(1).await
    }

    /// Waits for `n` slots at once.
    ///
    /// # Errors
    /// When the slots are closed.
    pub async fn take_many(&self, n: u32) -> Result<Slot, tokio::sync::AcquireError> {
        let permit = Arc::clone(&self.sem).acquire_many_owned(n).await?;
        Ok(Slot { permit: Some(permit), slots: self.clone() })
    }

    /// One slot if one is free now.
    #[must_use]
    pub fn try_take(&self) -> Option<Slot> {
        let permit = Arc::clone(&self.sem).try_acquire_owned().ok()?;
        Some(Slot { permit: Some(permit), slots: self.clone() })
    }

    /// Slots free right now.
    #[must_use]
    pub fn available_permits(&self) -> usize {
        self.sem.available_permits()
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let Some(permit) = self.permit.take() else { return };
        let n = permit.num_permits();
        let Ok(mut st) = self.slots.state.lock() else { return };
        let pay = st.owed.min(n);
        if pay == 0 {
            return;
        }
        // Forgotten under the lock, before the rest returns: no waiter can take them first.
        permit.forget();
        st.owed -= pay;
        st.cap -= pay;
        self.slots.sem.add_permits(n - pay);
    }
}
