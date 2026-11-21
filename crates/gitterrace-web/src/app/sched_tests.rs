use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::{Scheduler, percent, share, tier};

/// Long enough to never be the wait a test depends on.
const LONG: Duration = Duration::from_secs(3600);
/// A deadline for a message that must arrive.
const DEADLINE: Duration = Duration::from_secs(60);

/// A scheduler reading `load`, polling every `poll`.
fn sched(load: &Arc<AtomicU32>, poll: Duration, idle: Duration) -> Arc<Scheduler> {
    let l = Arc::clone(load);
    Scheduler::with(Box::new(move || Some(l.load(Ordering::SeqCst))), poll, idle)
}

fn sampling(s: &Scheduler) -> bool {
    s.lock().sampling
}

/// Guard class: the load tiers. 12 under 60, 4 up to 100, one above, at each edge. Red when
/// a tier compares with the wrong operator or a load that cannot be read stops the builds.
#[test]
fn the_budget_is_twelve_under_sixty_four_up_to_a_hundred_one_above() {
    let at = |l| tier(Some(l));
    assert_eq!([at(0), at(50), at(59)], [12, 12, 12]);
    assert_eq!([at(60), at(80), at(100)], [4, 4, 4]);
    assert_eq!([at(101), at(120)], [1, 1]);
    assert_eq!(tier(None), 12, "an unreadable load does not stop builds");
}

#[test]
fn builds_split_the_budget_and_none_is_left_idle_beside_one_that_runs() {
    assert_eq!([share(12, 1), share(12, 2), share(12, 5)], [12, 6, 3]);
    assert_eq!(share(4, 5), 1);
    assert_eq!(share(0, 2), 0);
}

/// Guard class: a running build follows the load. Starts at the tier of the load it entered
/// under and is sent each change after. Red when the sampler never reports.
#[test]
fn a_running_build_is_sent_the_budget_of_each_load_it_sees() {
    let load = Arc::new(AtomicU32::new(50));
    let s = sched(&load, Duration::from_millis(1), LONG);
    let (start, _lease, rx) = s.enter();
    assert_eq!(start, 12);
    load.store(80, Ordering::SeqCst);
    assert_eq!(rx.recv_timeout(DEADLINE), Ok(4));
    load.store(120, Ordering::SeqCst);
    assert_eq!(rx.recv_timeout(DEADLINE), Ok(1));
    load.store(10, Ordering::SeqCst);
    assert_eq!(rx.recv_timeout(DEADLINE), Ok(12));
}

/// Guard class: a second build takes half of the first's workers at once, and a build that
/// ends gives its share back. Red when budgets are only recomputed on a poll.
#[test]
fn a_build_entering_or_ending_moves_the_share_of_the_others() {
    let load = Arc::new(AtomicU32::new(0));
    let s = sched(&load, LONG, LONG);
    let (first, _a, rx) = s.enter();
    assert_eq!(first, 12);
    let (second, b, _rx2) = s.enter();
    assert_eq!(second, 6);
    assert_eq!(rx.recv_timeout(DEADLINE), Ok(6));
    drop(b);
    assert_eq!(rx.recv_timeout(DEADLINE), Ok(12));
}

/// Guard class: the idle pool exit. The sampler thread ends once no build ran for the idle
/// window, and a later build starts it again. Red when the sampler outlives its builds.
#[test]
fn the_sampler_exits_when_idle_and_starts_again_with_the_next_build() {
    let load = Arc::new(AtomicU32::new(0));
    let s = sched(&load, LONG, Duration::from_millis(1));
    let (_, lease, _rx) = s.enter();
    assert!(sampling(&s));
    drop(lease);
    gitterrace_testkit::wait::until_blocking("sampler exits", || !sampling(&s));
    let (start, _lease, _rx) = s.enter();
    assert_eq!(start, 12);
    assert!(sampling(&s), "a new build restarts the sampler");
}

/// Guard class: a build reads the load others make, not its own. Twelve workers on ten cores
/// are 120% of the load average a minute on, which tiered a lone build down to one worker
/// (cold rust index 63 s then 212 s). Red when the budget is read off the raw load.
#[test]
fn a_lone_build_is_not_throttled_by_the_load_its_own_workers_make() {
    let own = percent(12.0, 10);
    assert_eq!(own, 120);
    let load = Arc::new(AtomicU32::new(own));
    let s = sched(&load, LONG, LONG);
    // Settled: the average has caught up with the workers granted.
    s.lock().own = 12.0;
    let (first, _a, _rx) = s.enter();
    s.lock().own = 12.0;
    let (_, _b, _rx2) = s.enter();
    let mut st = s.lock();
    let each = share(tier(Some(own.saturating_sub(st.own_percent()))), st.builds.len());
    assert_eq!(first, 12);
    assert_eq!(each, 6, "the own load is subtracted, so two builds still split twelve");
}
