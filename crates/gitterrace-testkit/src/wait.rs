//! Waiting in tests on the thing itself, never on the clock.
//!
//! A test that sleeps and then asserts is a bet on machine speed: a loaded runner loses it. A
//! test asks [`until`] for the condition it means (the rebuild is done, the task let go), with
//! a deadline far past any healthy run, and fails with what it waited for. The deadline is the
//! only time in it: a healthy run returns as soon as the condition holds.
//!
//! Two ways to run past a slow machine without a longer clock. `TEST_WAIT_SCALE=<n>` multiplies
//! [`DEADLINE`] for a runner known to be slower still ([`deadline`]). [`until_moving`] counts the
//! deadline from the last change of the state it watches, so a wait that keeps progressing is
//! never cut off and a stuck one fails naming the state it stopped in.
//!
//! `tests/conventions.rs` refuses a raw sleep in test code outside this file, so the next wait
//! goes through here too.

use std::fmt::Debug;
use std::future::Future;
use std::time::{Duration, Instant};

/// Far past any healthy wait seen, loaded runners included: only a hang reaches it.
pub const DEADLINE: Duration = Duration::from_secs(120);

/// How often a condition is asked again.
const STEP: Duration = Duration::from_millis(20);

/// A moving wait ends at this many idle limits even when its state keeps changing, so a state
/// that changes on its own (a clock in a body) cannot hold a test forever. Not configurable.
const HARD_CAP: u32 = 10;

/// The multiplier `TEST_WAIT_SCALE` names, 1 when unset or not a positive whole number.
fn scale(raw: Option<&str>) -> u32 {
    raw.and_then(|v| v.trim().parse::<u32>().ok()).filter(|n| *n > 0).unwrap_or(1)
}

/// [`DEADLINE`] times `TEST_WAIT_SCALE`: what every hang wait here allows.
#[must_use]
pub fn deadline() -> Duration {
    DEADLINE.saturating_mul(scale(std::env::var("TEST_WAIT_SCALE").ok().as_deref()))
}

/// Awaits `cond` until it holds, at most [`deadline`]. Panics naming `what` when it never does.
pub async fn until<F, Fut>(what: &str, cond: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let limit = deadline();
    assert!(within(limit, cond).await, "{what}: not within {limit:?}");
}

/// Awaits a probe answering `(done, state)` until `done`, and returns the state it was done in.
///
/// `idle` runs from the last change of the state, not from the start: a build that keeps
/// advancing is waited for however long it takes (up to ten times `idle`), and one
/// that stops moving fails after `idle` with the last state in the message.
pub async fn until_moving<S, F, Fut>(idle: Duration, what: &str, mut probe: F) -> S
where
    S: PartialEq + Debug,
    F: FnMut() -> Fut,
    Fut: Future<Output = (bool, S)>,
{
    let start = Instant::now();
    let mut changed = start;
    let mut last: Option<S> = None;
    loop {
        let (done, state) = probe().await;
        if done {
            return state;
        }
        let now = Instant::now();
        if last.as_ref() != Some(&state) {
            changed = now;
        }
        last = Some(state);
        assert!(
            now.duration_since(changed) < idle && now.duration_since(start) < idle * HARD_CAP,
            "{what}: no progress within {idle:?}, waited {:?}; last state: {last:?}",
            now.duration_since(start)
        );
        pause(STEP).await;
    }
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
/// Prefixed as [`crate::page`] does: the wait for a page to show data instead of the indexing
/// progress. The [`deadline`] runs from the last change of the body, so a progress body that
/// advances is waited for. Panics with the last body.
pub async fn page_until(router: &axum::Router, uri: &str, ok: impl Fn(&str) -> bool) -> String {
    let ok = &ok;
    until_moving(deadline(), uri, || async move {
        let (_, body) = crate::page(router, uri).await;
        (ok(&body), body)
    })
    .await
}

/// A page body that holds data, not the indexing progress.
#[must_use]
pub fn indexed(body: &str) -> bool {
    !body.contains("data-progress")
}

/// A page body whose chrome already shows the repo id.
///
/// The id is computed once in the background, so a page rendered before it is known has
/// `data-id=""` and differs byte for byte from the same page a moment later. Wait for this
/// before comparing two whole pages.
#[must_use]
pub fn identified(body: &str) -> bool {
    !body.contains("data-id=\"\" data-link")
}

/// The blocking twin of [`until`], for tests without a runtime.
pub fn until_blocking(what: &str, mut cond: impl FnMut() -> bool) {
    let limit = deadline();
    let end = Instant::now() + limit;
    while !cond() {
        assert!(Instant::now() < end, "{what}: not within {limit:?}");
        std::thread::sleep(STEP);
    }
}

/// What `ps` says of process `pid`: its state letters, `None` once it is gone.
///
/// A process killed but not yet reaped still answers `kill -0` and shows as a zombie (`Z`):
/// where nothing reaps orphans it stays so, which is no running process.
#[must_use]
pub fn process_state(pid: &str) -> Option<String> {
    let out =
        std::process::Command::new("ps").args(["-o", "stat=", "-p", pid.trim()]).output().ok()?;
    let stat = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !stat.is_empty()).then_some(stat)
}

/// Waits until process `pid` is gone or a zombie, and fails naming its last state.
pub fn process_gone_blocking(what: &str, pid: &str) {
    let limit = deadline();
    let end = Instant::now() + limit;
    loop {
        let state = process_state(pid);
        if state.as_deref().is_none_or(|s| s.starts_with('Z')) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{what}: process {pid} not gone within {limit:?}; state {state:?}"
        );
        std::thread::sleep(STEP);
    }
}

/// A deliberate pause: a quiet window a test asserts nothing happens in, or pacing between
/// writes. Never a wait for something to happen; that is [`until`].
pub async fn pause(d: Duration) {
    tokio::time::sleep(d).await;
}

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

    /// Guard class: a wait that cuts a progressing build off at its base limit. Red when the
    /// limit counts from the start: the probe's state changes every step for far longer.
    #[tokio::test]
    async fn a_wait_that_keeps_progressing_is_not_cut_off_at_the_idle_limit() {
        let idle = Duration::from_millis(300);
        let t0 = Instant::now();
        let mut n = 0_u32;
        let last = until_moving(idle, "progress", || {
            n += 1;
            let n = n;
            async move { (n >= 40, n) }
        })
        .await;
        assert_eq!(last, 40);
        assert!(t0.elapsed() > idle, "the wait ended inside the idle limit");
    }

    /// Guard class: a stuck wait that passes silently or fails without saying where it stood.
    #[tokio::test]
    async fn a_stuck_wait_fails_with_its_last_state() {
        let stuck = tokio::spawn(until_moving(Duration::from_millis(100), "the build", || async {
            (false, "done 0/2")
        }));
        let err = stuck.await.expect_err("a stuck wait must panic");
        assert!(err.is_panic());
        let payload = err.into_panic();
        let msg = payload.downcast_ref::<String>().expect("a formatted message");
        assert!(msg.contains("the build"), "{msg}");
        assert!(msg.contains("done 0/2"), "{msg}");
    }

    #[test]
    fn the_scale_is_a_positive_whole_number_or_one() {
        let at = |v| scale(Some(v));
        assert_eq!([at("3"), at(" 2 "), at("0"), at("x"), at("-1")], [3, 2, 1, 1, 1]);
        assert_eq!(scale(None), 1);
    }

    #[test]
    fn a_missing_process_is_gone() {
        assert_eq!(process_state("2147483646"), None);
        process_gone_blocking("a process that never existed", "2147483646");
    }

    #[tokio::test]
    async fn a_condition_never_met_ends_at_the_limit() {
        let t0 = Instant::now();
        assert!(!within(Duration::from_millis(100), || async { false }).await);
        assert!(t0.elapsed() >= Duration::from_millis(100));
    }
}
