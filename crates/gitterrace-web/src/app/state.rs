//! Where a repository is on its way to serving insights: one value the progress endpoint, the
//! pending page and the repository list all read, so none of them can say ready while the
//! index or the warm-up still runs.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::app::App;

/// A repository's insights state. Code and commits serve in every state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RepoState {
    /// Waiting for its first index build: not started yet, or `index.build_on_start` is off
    /// and no page asked for insights.
    Queued,
    /// The index build runs.
    Indexing,
    /// The index holds HEAD; the tables the first pages read are being built.
    Warming,
    /// Insights pages serve.
    Ready,
    /// The last build attempt failed before the repository was ever ready. The next change
    /// of HEAD, or a page that asks, tries again.
    Failed,
}

impl RepoState {
    const ALL: [Self; 5] = [
        Self::Queued,
        Self::Indexing,
        Self::Warming,
        Self::Ready,
        Self::Failed,
    ];

    /// The word the pages show, the same as the API value.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Indexing => "indexing",
            Self::Warming => "warming",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    const fn code(self) -> u8 {
        self as u8
    }

    fn of(code: u8) -> Self {
        Self::ALL
            .into_iter()
            .find(|s| s.code() == code)
            .unwrap_or(Self::Queued)
    }
}

/// The state of one app, shared by its refresher and its pages. Starts queued: the default
/// code is [`RepoState::Queued`]'s.
#[derive(Debug, Default)]
pub(crate) struct StateCell {
    now: AtomicU8,
}

impl StateCell {
    pub(crate) fn get(&self) -> RepoState {
        RepoState::of(self.now.load(Ordering::Acquire))
    }

    /// Moves to `to` unless the repository is ready: a later update or its failure leaves a
    /// ready repository serving what it has. Returns whether it moved.
    pub(crate) fn advance(&self, to: RepoState) -> bool {
        self.now
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |c| {
                (c != RepoState::Ready.code()).then_some(to.code())
            })
            .is_ok()
    }

    /// Warming to ready, and nothing else. Returns whether it moved.
    fn finish_warming(&self) -> bool {
        self.now
            .compare_exchange(
                RepoState::Warming.code(),
                RepoState::Ready.code(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Moves to `to` from any state: ready once the warm-up is done, queued again when the
    /// cache is rebuilt.
    pub(crate) fn set(&self, to: RepoState) {
        self.now.store(to.code(), Ordering::Release);
    }
}

impl App {
    /// True once one full pass over HEAD and its warm-up finished, so records can be trusted.
    #[cfg(test)]
    pub(crate) fn ready(&self) -> bool {
        self.state.get() == RepoState::Ready
    }

    pub(crate) fn state(&self) -> RepoState {
        self.state.get()
    }

    /// Moves a repository not yet ready on to `to`, see [`StateCell::advance`].
    pub(crate) fn advance(&self, to: RepoState) -> bool {
        self.state.advance(to)
    }

    /// Ready at once, for tests that build the index by hand.
    #[cfg(test)]
    pub(crate) fn mark_ready(&self) {
        self.state.set(RepoState::Ready);
    }

    /// Ready once the warm-up is done, but only from warming: a rebuild that put the
    /// repository back to queued meanwhile keeps it there.
    pub(crate) fn finish_warming(&self) -> bool {
        self.state.finish_warming()
    }

    /// Whether insights pages may read the index: it holds HEAD, so the warm-up renders
    /// them, and a visitor while it runs gets the same page.
    pub(crate) fn insights_open(&self) -> bool {
        matches!(self.state.get(), RepoState::Warming | RepoState::Ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_reads_back_as_written() {
        assert_eq!(RepoState::Queued.code(), 0, "the default cell is queued");
        for s in RepoState::ALL {
            assert_eq!(RepoState::of(s.code()), s);
            let json = serde_json::to_string(&s).expect("json");
            assert_eq!(json, format!("\"{}\"", s.label()));
        }
    }

    /// A ready repository stays ready through a later update and its failure; only an
    /// explicit reset (a rebuild) takes it back.
    #[test]
    fn a_ready_repository_stays_ready_until_reset() {
        let c = StateCell::default();
        assert_eq!(c.get(), RepoState::Queued);
        assert!(c.advance(RepoState::Indexing));
        assert!(c.advance(RepoState::Warming));
        assert_eq!(c.get(), RepoState::Warming);
        c.set(RepoState::Ready);
        assert!(!c.advance(RepoState::Indexing));
        assert!(!c.advance(RepoState::Failed));
        assert_eq!(c.get(), RepoState::Ready);
        c.set(RepoState::Queued);
        assert_eq!(c.get(), RepoState::Queued);
        c.finish_warming();
        assert_eq!(
            c.get(),
            RepoState::Queued,
            "a rebuild during the warm-up stays queued"
        );
        assert!(c.advance(RepoState::Warming));
        c.finish_warming();
        assert_eq!(c.get(), RepoState::Ready);
    }

    /// Guard class: any path that reports ready before the index holds HEAD, whether a
    /// flag set early, a state read from the wrong place or a progress body that derives
    /// ready on its own. The build is held at the cache's writer lock, so it sits mid-update
    /// for as long as the test needs: queued before the demand, indexing while held, ready
    /// only after.
    #[tokio::test]
    async fn progress_is_never_ready_while_the_index_builds() {
        use crate::wiring_tests::{app, repo, with};
        let dir = tempfile::tempdir().expect("tmp");
        let p = repo(dir.path(), "r");
        let a = app(&p, with(|c| c.index.build_on_start = false));
        let router = crate::routes::router(std::sync::Arc::clone(&a));
        // Each call owns its router handle, so the futures borrow nothing from the test.
        let state = || {
            let r = router.clone();
            async move {
                let (_, body) = gitterrace_testkit::get(&r, "/api/progress").await;
                let v: serde_json::Value = serde_json::from_str(&body).expect("json");
                (
                    v["state"].as_str().unwrap_or("").to_owned(),
                    v["ready"] == true,
                )
            }
        };
        tokio::spawn(crate::app::keep_index_fresh_supervised(
            std::sync::Arc::clone(&a),
        ));
        assert_eq!(state().await, ("queued".to_owned(), false));
        let lock = std::fs::File::create(a.index.dir().join("lock")).expect("lock file");
        lock.lock().expect("hold the writer lock");
        a.demand();
        gitterrace_testkit::wait::until("the build starts", || {
            let f = state();
            async move { f.await.0 == "indexing" }
        })
        .await;
        // The update waits on the lock: however often it is asked, progress says indexing.
        for _ in 0..20 {
            assert_eq!(state().await, ("indexing".to_owned(), false));
        }
        assert!(!a.ready());
        lock.unlock().expect("release");
        gitterrace_testkit::wait::until("the build finishes", || {
            let f = state();
            async move { f.await == ("ready".to_owned(), true) }
        })
        .await;
        let head = a.repo.resolve("HEAD").expect("head");
        assert_eq!(
            a.index.pending(&a.repo, &head).expect("pending"),
            0,
            "ready before the index held HEAD"
        );
    }

    /// A build that cannot run (a cache a newer build owns) reports failed, never ready and
    /// never stuck at indexing, so a reader waiting on the state stops waiting.
    #[tokio::test]
    async fn a_build_that_cannot_run_reports_failed() {
        use crate::wiring_tests::{repo, with};
        let dir = tempfile::tempdir().expect("tmp");
        let p = repo(dir.path(), "r");
        let cache = p.join(".git/gitterrace");
        std::fs::create_dir_all(&cache).expect("cache dir");
        std::fs::write(cache.join("VERSION"), "v99\n").expect("stamp");
        let a = crate::wiring_tests::app(&p, with(|_| {}));
        tokio::spawn(crate::app::keep_index_fresh_supervised(
            std::sync::Arc::clone(&a),
        ));
        gitterrace_testkit::wait::until("the build gives up", || async {
            a.state() == RepoState::Failed
        })
        .await;
        let router = crate::routes::router(std::sync::Arc::clone(&a));
        let (_, body) = gitterrace_testkit::get(&router, "/api/progress").await;
        assert!(
            body.contains("\"state\":\"failed\"") && body.contains("\"ready\":false"),
            "{body}"
        );
    }
}
