//! Running page work off the async runtime, under the page job slots and with the
//! request's display context in force.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::app::App;
use crate::error::Result;

thread_local! {
    static CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Raises the flag when dropped: the response went away, finished or not.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The flag of the job running on this thread, raised once its request is dropped. Work that
/// can run long on hostile input polls it. A thread outside a job gets a flag never raised.
pub(crate) fn cancel_flag() -> Arc<AtomicBool> {
    CANCEL.with(|c| c.borrow().clone()).unwrap_or_default()
}

pub(crate) async fn blocking<T: Send + 'static>(
    app: &Arc<App>,
    f: impl FnOnce(&App) -> Result<T> + Send + 'static,
) -> Result<T> {
    let app = Arc::clone(app);
    let rev = crate::fmt::REQUEST_REV.try_with(Clone::clone).unwrap_or_else(|_| "HEAD".to_owned());
    let switch =
        crate::fmt::REQUEST_SWITCH.try_with(Clone::clone).unwrap_or_else(|_| "/".to_owned());
    // Moved into the job, so a cancelled request cannot free the slot while git still runs.
    let slot = Arc::clone(&app.env.jobs)
        .acquire_owned()
        .await
        .map_err(|_| std::io::Error::other("blocking job slots closed"))?;
    let cfg = app.cfg();
    let absolute = cfg.display.dates == gitterrace_types::DateStyle::Absolute;
    // Every page is built here, so every page reads one table of what is switched on.
    let on = crate::features::Effective::of(&cfg, Some(&app.name));
    // Held by this future: dropping it (a public timeout, a gone client) raises the flag the
    // running job polls, since a blocking job cannot be aborted.
    let cancel = Arc::new(AtomicBool::new(false));
    let _gone = CancelOnDrop(Arc::clone(&cancel));
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        CANCEL.with(|c| *c.borrow_mut() = Some(cancel));
        let out = crate::features::with(on, || {
            crate::fmt::with_dates(absolute, || {
                crate::fmt::with_base(&app.base, || {
                    crate::fmt::with_rev(&rev, || crate::fmt::with_switch(&switch, || f(&app)))
                })
            })
        });
        // The pool thread outlives the job: a later job must not see this one's flag.
        CANCEL.with(|c| *c.borrow_mut() = None);
        out
    })
    .await?
}
