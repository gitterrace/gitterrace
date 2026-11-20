//! OS file events for the refresher, one core for both kinds. The working tree watch
//! ([`Scope::Tree`]) runs while a tab listens and is dropped when the last one leaves, so an idle
//! server holds no tree watches. The git directories watch ([`Scope::Git`]) runs while the
//! watcher is on, tab or not. A nudge only says "check again"; the check stays the truth.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use gitterrace_git::{Nudge, NudgeError, Nudger, Repo};
use tokio::sync::Notify;

use crate::app::App;

/// How often HEAD, the refs and the index are checked, and the working tree scanned while a tab
/// shows it, where OS file events are unavailable (the watch limit, some network volumes). Not
/// configurable: it only applies where events fail, a check is a few `stat` calls, and with no
/// tab open the check waits for `watch.rescan_secs` instead.
pub(crate) const POLL_FALLBACK: Duration = Duration::from_secs(1);

/// What a watch covers. The only thing the two kinds differ in: which folders and the words of
/// the log lines. The state machine is [`Watch`] for both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Tree,
    /// HEAD, refs, `packed-refs` and the index. A handful of watches, woken only by a git write.
    Git,
}

impl Scope {
    fn start(
        self,
        repo: &Repo,
        rescan: Duration,
        os: bool,
        on: impl Fn(Nudge) + Send + 'static,
    ) -> Result<Nudger, NudgeError> {
        match self {
            Self::Tree => repo.file_events(rescan, os, on),
            Self::Git => repo.git_events(rescan, os, on),
        }
    }

    const fn what(self) -> &'static str {
        match self {
            Self::Tree => "changes",
            Self::Git => "new commits",
        }
    }
}

/// The nudge state of one watch of one repository's refresher.
pub(crate) struct Watch {
    scope: Scope,
    run: Option<Arc<Run>>,
    /// Set by a nudge, cleared by the check that answers it.
    pending: Arc<AtomicBool>,
    pub(crate) nudged: Arc<Notify>,
    /// Events could not start or stopped being trusted: check on a timer, as polling.
    polling: Arc<AtomicBool>,
    started: Option<(u64, bool)>,
}

/// One start of a watcher. Registering the OS watches took up to twenty seconds on a loaded
/// Mac, so it runs off the refresher: until it is up the refresher polls, and a start that
/// finishes after its watch was stopped is retired at once.
#[derive(Default)]
struct Run {
    nudger: Mutex<Option<Nudger>>,
    /// The start is still registering its watches: no event can arrive yet.
    starting: AtomicBool,
    /// Stopped or restarted meanwhile: whatever the start returns is retired.
    stopped: AtomicBool,
}

impl Run {
    /// Keeps the started `nudger`, or retires it when the watch was stopped meanwhile.
    fn up(&self, nudger: Option<Nudger>) {
        if let Ok(mut g) = self.nudger.lock() {
            if self.stopped.load(Ordering::Acquire) {
                retire(nudger);
            } else {
                *g = nudger;
            }
        }
        self.starting.store(false, Ordering::Release);
    }

    fn stop(&self) {
        if let Ok(mut g) = self.nudger.lock() {
            self.stopped.store(true, Ordering::Release);
            retire(g.take());
        }
    }

    #[cfg(test)]
    fn running(&self) -> bool {
        self.nudger.lock().is_ok_and(|g| g.is_some())
    }
}

impl Watch {
    pub(crate) fn new(scope: Scope) -> Self {
        Self {
            scope,
            run: None,
            pending: Arc::default(),
            nudged: Arc::default(),
            polling: Arc::default(),
            started: None,
        }
    }

    /// Runs the watcher with `rescan_secs`, or stops it for `None`. A change of settings
    /// restarts it. Starting walks the folders once (one watch each on Linux) and registers
    /// the OS stream, which a busy machine took seconds for: it runs on a thread of its own and
    /// the refresher polls until it is up, so a change meanwhile still shows within the bound.
    pub(crate) fn follow(&mut self, app: &Arc<App>, rescan_secs: Option<u64>) {
        let os = app.os_events.load(Ordering::Acquire);
        let want = rescan_secs.map(|s| (s, os));
        if want == self.started {
            return;
        }
        if let Some(r) = self.run.take() {
            r.stop();
        }
        self.polling.store(false, Ordering::Release);
        self.started = want;
        let Some((secs, os)) = want else { return };
        let scope = self.scope;
        self.start(app, os, move |a, on| scope.start(&a.repo, Duration::from_secs(secs), os, on));
    }

    /// Runs `start` on a blocking thread. With OS events (`os`) the watch polls until it is
    /// up and looks once more then; without, the safety timer is the only check, as before.
    fn start<F>(&mut self, app: &Arc<App>, os: bool, start: F)
    where
        F: FnOnce(&App, Box<dyn Fn(Nudge) + Send>) -> Result<Nudger, NudgeError> + Send + 'static,
    {
        // The first check after a start sets the baseline.
        self.pending.store(true, Ordering::Release);
        let run = Arc::new(Run::default());
        run.starting.store(os, Ordering::Release);
        self.run = Some(Arc::clone(&run));
        let (a, scope) = (Arc::clone(app), self.scope);
        let on: Box<dyn Fn(Nudge) + Send> = Box::new(self.on_nudge(app.repo.name()));
        let (polling, pending) = (Arc::clone(&self.polling), Arc::clone(&self.pending));
        let nudged = Arc::clone(&self.nudged);
        tokio::task::spawn_blocking(move || {
            let r = start(&a, on);
            if let Err(e) = &r {
                let (what, name) = (scope.what(), a.repo.name());
                tracing::warn!("{e}; checking {name} for {what} by polling");
                polling.store(true, Ordering::Release);
            }
            run.up(r.ok());
            if os {
                // A change between the last poll and the first watch has no event: look again.
                pending.store(true, Ordering::Release);
                nudged.notify_one();
            }
        });
    }

    /// What the watcher thread calls on a nudge: mark it, switch to polling on a degraded watch,
    /// wake the refresher.
    fn on_nudge(&self, name: String) -> impl Fn(Nudge) + Send + 'static {
        let (pending, nudged) = (Arc::clone(&self.pending), Arc::clone(&self.nudged));
        let (polling, what) = (Arc::clone(&self.polling), self.scope.what());
        move |n: Nudge| {
            if n == Nudge::Degraded {
                tracing::warn!(
                    "file events stopped for {name}: too many folders or the watch limit; \
                     checking for {what} by polling"
                );
                polling.store(true, Ordering::Release);
            }
            pending.store(true, Ordering::Release);
            nudged.notify_one();
        }
    }

    /// Whether a check should run this round, and clears the nudge.
    pub(crate) fn take(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel) || self.polling()
    }

    /// Whether a scan is owed, without clearing it.
    pub(crate) fn pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    /// Whether checks run on a timer because events are unavailable.
    pub(crate) fn polling(&self) -> bool {
        self.polling.load(Ordering::Acquire)
            || self.run.as_ref().is_some_and(|r| r.starting.load(Ordering::Acquire))
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(r) = self.run.take() {
            r.stop();
        }
    }
}

/// Stops `nudger` on a thread of its own. Dropping it joins the watcher thread, and the OS
/// stream's teardown took up to forty seconds on a loaded Mac: run on an async worker (where a
/// refresher is dropped when its repository is removed) that stalled the worker and kept the
/// whole app alive meanwhile. The nudger holds no app, so nothing waits for this thread.
fn retire<T: Send + 'static>(nudger: Option<T>) {
    let Some(n) = nudger else { return };
    let spawned = std::thread::Builder::new().name("fs-nudge-stop".into()).spawn(move || drop(n));
    if let Err(e) = spawned {
        tracing::warn!("stopping a file watcher on its own thread failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::Ordering};

    use gitterrace_git::Nudge;

    use super::{Scope, Watch};
    use crate::wiring_tests::{app, repo, with};

    const BOTH: [Scope; 2] = [Scope::Tree, Scope::Git];

    /// Records the thread it is dropped on.
    struct Spy(std::sync::mpsc::Sender<std::thread::ThreadId>);

    impl Drop for Spy {
        fn drop(&mut self) {
            self.0.send(std::thread::current().id()).ok();
        }
    }

    /// Guard class: a blocking teardown run on the caller's thread. A watcher's drop joins the
    /// OS stream's thread, which took tens of seconds on a loaded Mac, and the caller is an
    /// async worker holding the app. Red when `retire` drops in place.
    #[test]
    fn a_retired_watcher_is_dropped_off_the_calling_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        super::retire(Some(Spy(tx)));
        let on = rx
            .recv_timeout(gitterrace_testkit::wait::deadline())
            .expect("the retired value was dropped");
        assert_ne!(on, std::thread::current().id(), "dropped on the caller");
    }

    /// Whether `w`'s watcher finished starting and is up.
    async fn up(w: &Watch) -> bool {
        gitterrace_testkit::wait::within(gitterrace_testkit::wait::deadline(), || async {
            w.run.as_ref().is_some_and(|r| r.running()) && !w.polling()
        })
        .await
    }

    /// Guard class: the two watches drifting apart. Each kind runs the same start, restart,
    /// stop and nudge steps; a kind that skipped one (kept polling after a stop, ignored a
    /// degraded nudge, did not restart on new settings) fails here for that kind.
    #[tokio::test]
    async fn both_watch_kinds_share_one_state_machine() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = app(&repo(dir.path(), "r"), with(|_| {}));
        a.os_events.store(false, Ordering::Release);
        for scope in BOTH {
            let mut w = Watch::new(scope);
            w.follow(&a, Some(5));
            assert!(up(&w).await, "{scope:?} did not start");
            assert!(w.take(), "{scope:?}: the first check is the baseline");
            assert!(!w.take(), "{scope:?}: the nudge was not cleared");
            let on = w.on_nudge("r".into());
            let woke = Arc::clone(&w.nudged);
            on(Nudge::Degraded);
            assert!(w.polling(), "{scope:?} ignored a degraded watch");
            assert!(w.take(), "{scope:?} lost the nudge");
            tokio::time::timeout(std::time::Duration::from_secs(1), woke.notified())
                .await
                .unwrap_or_else(|_| panic!("{scope:?} did not wake the refresher"));
            w.follow(&a, Some(6));
            assert_eq!(w.started, Some((6, false)), "{scope:?} kept old settings");
            assert!(up(&w).await, "{scope:?} kept polling after a restart");
            w.follow(&a, None);
            assert!(w.run.is_none() && !w.polling(), "{scope:?} did not stop");
        }
    }

    /// Guard class: the refresher waiting on a watcher start. Registering the OS watches took
    /// twenty seconds on a loaded Mac, and every change made meanwhile waited for it. A start
    /// that blocks here must leave `follow` returned at once and the watch polling, so a check
    /// still runs every poll interval; a start that is stopped before it is up is never kept.
    #[tokio::test]
    async fn a_slow_watcher_start_polls_until_it_is_up() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = app(&repo(dir.path(), "r"), with(|_| {}));
        for scope in BOTH {
            let mut w = Watch::new(scope);
            let (go, wait) = std::sync::mpsc::channel::<()>();
            w.start(&a, true, move |a, on| {
                wait.recv().ok();
                a.repo.git_events(std::time::Duration::from_secs(5), false, on)
            });
            assert!(w.polling(), "{scope:?}: no check runs while the start is slow");
            go.send(()).expect("release the start");
            assert!(up(&w).await, "{scope:?}: the start never came up");
            assert!(w.take(), "{scope:?}: no look once the watch is up");
            let (go, wait) = std::sync::mpsc::channel::<()>();
            w.start(&a, true, move |a, on| {
                wait.recv().ok();
                a.repo.git_events(std::time::Duration::from_secs(5), false, on)
            });
            let late = w.run.clone().expect("a run");
            // As if `follow` had started it: the stop below is a change of settings.
            w.started = Some((5, false));
            w.follow(&a, None);
            go.send(()).expect("release the start");
            let kept =
                gitterrace_testkit::wait::within(std::time::Duration::from_secs(2), || async {
                    late.running()
                })
                .await;
            assert!(!kept, "{scope:?}: a start stopped meanwhile was kept");
        }
    }
}
