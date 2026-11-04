//! OS file events for the refresher: started while a tab listens, dropped when the last one
//! leaves, so an idle server holds no watches. A nudge only says "scan again"; the scan stays
//! the truth.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use gitterrace_git::{Nudge, Nudger};
use tokio::sync::Notify;

use crate::app::App;

/// The nudge state of one repository's refresher.
#[derive(Default)]
pub(crate) struct Events {
    nudger: Option<Nudger>,
    /// Set by a nudge, cleared by the scan that answers it.
    pending: Arc<AtomicBool>,
    /// Wakes the refresher on a nudge.
    pub(crate) nudged: Arc<Notify>,
    /// File events could not start or stopped being trusted: scan every interval, as polling.
    polling: Arc<AtomicBool>,
    /// The settings the running watcher was started with.
    started: Option<(u64, bool)>,
}

impl Events {
    /// Starts or stops the watcher so it runs exactly while a tab listens. Starting walks the
    /// folders once (one watch each on Linux), so it runs off the async threads.
    pub(crate) async fn follow(&mut self, app: &Arc<App>, listening: bool, rescan_secs: u64) {
        let os = app.os_events.load(Ordering::Acquire);
        let want = listening.then_some((rescan_secs, os));
        if want == self.started {
            return;
        }
        // Dropping joins the watcher thread, which takes at most one debounce round.
        self.nudger = None;
        self.polling.store(false, Ordering::Release);
        self.started = want;
        let Some((secs, os)) = want else { return };
        // The first scan after a start sets the baseline.
        self.pending.store(true, Ordering::Release);
        let (a, pending, nudged, polling) = (
            Arc::clone(app),
            Arc::clone(&self.pending),
            Arc::clone(&self.nudged),
            Arc::clone(&self.polling),
        );
        let started = tokio::task::spawn_blocking(move || {
            let (degraded, name) = (Arc::clone(&polling), a.repo.name());
            let on = move |n: Nudge| {
                if n == Nudge::Degraded {
                    tracing::warn!(
                        "file events stopped for {name}: the watch limit; polling instead"
                    );
                    degraded.store(true, Ordering::Release);
                }
                pending.store(true, Ordering::Release);
                nudged.notify_one();
            };
            let r = a.repo.file_events(Duration::from_secs(secs), os, on);
            if let Err(e) = &r {
                tracing::warn!("{e}; checking {} for changes by polling", a.repo.name());
                polling.store(true, Ordering::Release);
            }
            r.ok()
        })
        .await;
        match started {
            Ok(n) => self.nudger = n,
            Err(_) => self.polling.store(true, Ordering::Release),
        }
    }

    /// Whether the working tree should be scanned this round, and clears the nudge.
    pub(crate) fn take(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel) || self.polling.load(Ordering::Acquire)
    }
}
