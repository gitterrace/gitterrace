//! OS file events for the refresher: started while a tab listens, dropped when the last one
//! leaves, so an idle server holds no watches. A nudge only says "scan again"; the scan stays
//! the truth. The git directories have their own watcher, [`GitEvents`], which runs while the
//! watcher is on, tab or not.

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
    /// File events could not start or stopped being trusted: scan on a timer, as polling.
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
                        "file events stopped for {name}: too many folders or the watch limit; \
                         relying on the periodic rescan"
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
        self.pending.swap(false, Ordering::AcqRel) || self.polling()
    }

    /// Whether a scan is owed, without clearing it.
    pub(crate) fn pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    /// Whether working tree scans run on a timer because events are unavailable.
    pub(crate) fn polling(&self) -> bool {
        self.polling.load(Ordering::Acquire)
    }
}

/// How often HEAD, the refs and the index are checked, and the working tree scanned while a tab
/// shows it, where OS file events are unavailable (the watch limit, some network volumes). Not
/// configurable: it only applies where events fail, a check is a few `stat` calls, and with no
/// tab open the check waits for `watch.rescan_secs` instead.
pub(crate) const POLL_FALLBACK: Duration = Duration::from_secs(1);

/// OS file events on the git directories: HEAD, refs, `packed-refs` and the index. Runs whenever
/// the watcher is on, tab or not, since it holds a handful of watches and wakes only on a git
/// write. Its safety timer, every `watch.rescan_secs`, bounds a dropped event.
#[derive(Default)]
pub(crate) struct GitEvents {
    nudger: Option<Nudger>,
    /// Wakes the refresher for a fingerprint check.
    pub(crate) nudged: Arc<Notify>,
    /// Events could not start or stopped being trusted: the caller polls.
    polling: Arc<AtomicBool>,
    started: Option<(u64, bool)>,
}

impl GitEvents {
    /// Starts the watcher with `rescan_secs`, or stops it for `None`. A change of settings
    /// restarts it.
    pub(crate) async fn follow(&mut self, app: &Arc<App>, rescan_secs: Option<u64>) {
        let os = app.os_events.load(Ordering::Acquire);
        let want = rescan_secs.map(|s| (s, os));
        if want == self.started {
            return;
        }
        self.nudger = None;
        self.polling.store(false, Ordering::Release);
        self.started = want;
        let Some((secs, os)) = want else { return };
        let (a, nudged, polling) = (
            Arc::clone(app),
            Arc::clone(&self.nudged),
            Arc::clone(&self.polling),
        );
        let started = tokio::task::spawn_blocking(move || {
            let degraded = Arc::clone(&polling);
            let on = move |n: Nudge| {
                if n == Nudge::Degraded {
                    degraded.store(true, Ordering::Release);
                }
                nudged.notify_one();
            };
            let r = a.repo.git_events(Duration::from_secs(secs), os, on);
            if let Err(e) = &r {
                tracing::warn!("{e}; checking {} for new commits by polling", a.repo.name());
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

    /// Whether the refresher must check on a timer because events are unavailable.
    pub(crate) fn polling(&self) -> bool {
        self.polling.load(Ordering::Acquire)
    }
}
