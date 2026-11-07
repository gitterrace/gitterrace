//! Waiting in tests on the thing itself, never on the clock.
//!
//! A test that sleeps and then asserts is a bet on machine speed: a loaded runner loses it. A
//! test asks [`until`] for the condition it means (the rebuild is done, the task let go), with
//! a deadline far past any healthy run, and fails with what it waited for. The deadline is the
//! only time in it: a healthy run returns as soon as the condition holds.
//!
//! `tests/conventions.rs` refuses a raw sleep in test code outside this file, so the next wait
//! goes through here too.

use std::time::{Duration, Instant};

/// Far past any healthy wait seen, loaded runners included: only a hang reaches it.
pub const DEADLINE: Duration = Duration::from_secs(120);

/// How often a condition is asked again.
const STEP: Duration = Duration::from_millis(20);

/// Awaits `cond` until it holds, at most [`DEADLINE`]. Panics naming `what` when it never does.
pub async fn until<F, Fut>(what: &str, cond: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    assert!(
        within(DEADLINE, cond).await,
        "{what}: not within {DEADLINE:?}"
    );
}

/// Awaits `cond` until it holds, at most `limit`, and says whether it did. For a test whose
/// promise is a bound (a change shows within the rescan): the bound is the assertion.
pub async fn within<F, Fut>(limit: Duration, mut cond: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let end = Instant::now() + limit;
    loop {
        if cond().await {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        pause(STEP).await;
    }
}

/// GETs page `uri` of the lone repository until its body passes `ok`, and returns it.
///
/// Prefixed as [`crate::page`] does, at most [`DEADLINE`]: the wait for a page to show data
/// instead of the indexing progress. Panics with the last body.
pub async fn page_until(router: &axum::Router, uri: &str, ok: impl Fn(&str) -> bool) -> String {
    let end = Instant::now() + DEADLINE;
    loop {
        let (_, body) = crate::page(router, uri).await;
        if ok(&body) {
            return body;
        }
        assert!(
            Instant::now() < end,
            "{uri} never answered as awaited within {DEADLINE:?}: {body}"
        );
        pause(STEP).await;
    }
}

/// A page body that holds data, not the indexing progress.
#[must_use]
pub fn indexed(body: &str) -> bool {
    !body.contains("data-progress")
}

/// The blocking twin of [`until`], for tests without a runtime.
pub fn until_blocking(what: &str, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + DEADLINE;
    while !cond() {
        assert!(Instant::now() < end, "{what}: not within {DEADLINE:?}");
        std::thread::sleep(STEP);
    }
}

/// A deliberate pause: a quiet window a test asserts nothing happens in, or pacing between
/// writes. Never a wait for something to happen; that is [`until`].
pub async fn pause(d: Duration) {
    tokio::time::sleep(d).await;
}

/// The blocking twin of [`pause`].
pub fn pause_blocking(d: Duration) {
    std::thread::sleep(d);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// A condition met later is awaited, not raced: red when `within` checks once.
    #[tokio::test]
    async fn a_condition_met_later_is_awaited() {
        let flag = Arc::new(AtomicBool::new(false));
        let f = Arc::clone(&flag);
        tokio::spawn(async move {
            pause(Duration::from_millis(100)).await;
            f.store(true, Ordering::Release);
        });
        let ok = within(Duration::from_secs(30), || {
            let f = Arc::clone(&flag);
            async move { f.load(Ordering::Acquire) }
        })
        .await;
        assert!(ok, "the flag set later was never seen");
    }

    #[tokio::test]
    async fn a_condition_never_met_ends_at_the_limit() {
        let t0 = Instant::now();
        assert!(!within(Duration::from_millis(100), || async { false }).await);
        assert!(t0.elapsed() >= Duration::from_millis(100));
    }
}
