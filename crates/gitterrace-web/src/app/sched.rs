//! The load budget of the index builds: how many workers the running builds may use, from the
//! machine's load.
//!
//! The index builds in a child process, so the server cannot cap its threads directly. It sends
//! the child `budget <n>` lines instead (see `child.rs`): 12 workers while the machine is under
//! 60% loaded, 4 up to 100%, one above, shared evenly by the builds running at once. A running
//! build keeps one worker even on a saturated machine, or it would never finish.
//!
//! One sampler thread reads the load while any build runs and exits after [`IDLE`] without
//! one, so an idle server keeps no thread for this.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Workers while the load is under [`BUSY`].
const FULL: usize = 12;
/// Workers while the load is up to [`SATURATED`].
const REDUCED: usize = 4;
/// Load, in percent of the cores, from which the budget drops to [`REDUCED`].
const BUSY: u32 = 60;
/// Workers above [`SATURATED`]: a floor, so a build on a busy machine still ends.
const FLOOR: usize = 1;
/// Load, in percent of the cores, above which only the [`FLOOR`] runs.
const SATURATED: u32 = 100;
/// The time constant of the system's 1-minute load average, which the builds' own share is
/// smoothed with to match.
const LOAD_WINDOW: f64 = 60.0;
const POLL: Duration = Duration::from_secs(5);
const IDLE: Duration = Duration::from_secs(30);

/// Set by a test that runs the real server: the load in percent, read instead of the machine's.
/// A build under a saturated machine pauses by design, which a test of the build itself must
/// not depend on.
const LOAD_VAR: &str = "TEST_INDEX_LOAD";

/// The machine's load in percent of its cores, `None` where it cannot be read.
type Load = dyn Fn() -> Option<u32> + Send + Sync;

/// The workers all builds may use together at `load` percent.
const fn tier(load: Option<u32>) -> usize {
    match load {
        None => FULL,
        Some(l) if l < BUSY => FULL,
        Some(l) if l <= SATURATED => REDUCED,
        Some(_) => FLOOR,
    }
}

/// `workers` running flat out, in percent of `cores`.
fn percent(workers: f64, cores: usize) -> u32 {
    // Saturating float to int: a share of the cores is small and never negative.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a percent")]
    let pct = (workers / cores.max(1) as f64 * 100.0).max(0.0) as u32;
    pct
}

/// One build's part of `total` workers among `builds`: even, and at least one while any
/// worker may run, so a build never idles beside another that has them.
const fn share(total: usize, builds: usize) -> usize {
    if total == 0 || builds == 0 { 0 } else { total.div_ceil(builds) }
}

struct Build {
    id: u64,
    tx: Sender<usize>,
    last: usize,
}

#[derive(Default)]
struct State {
    builds: Vec<Build>,
    /// The workers the builds themselves put on the load average, smoothed over
    /// [`LOAD_WINDOW`] as the average is, and when it was last updated. A build must not
    /// throttle itself for the load its own workers make.
    own: f64,
    own_at: Option<Instant>,
    next: u64,
    sampling: bool,
}

pub(crate) struct Scheduler {
    load: Box<Load>,
    poll: Duration,
    idle: Duration,
    state: Mutex<State>,
    entered: Condvar,
}

/// A running build's place in the scheduler. Dropping it frees the share for the others.
pub(crate) struct Lease {
    id: u64,
    sched: Arc<Scheduler>,
}

impl State {
    /// Smooths the workers granted so far into [`State::own`] and returns them as a percent of
    /// the cores.
    fn own_percent(&mut self) -> u32 {
        let now = Instant::now();
        let granted: usize = self.builds.iter().map(|b| b.last).sum();
        let dt = self.own_at.map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.own = (granted as f64 - self.own).mul_add(1.0 - (-dt / LOAD_WINDOW).exp(), self.own);
        self.own_at = Some(now);
        percent(self.own, std::thread::available_parallelism().map_or(1, usize::from))
    }
}

impl Scheduler {
    /// The scheduler of a server: the system load, polled every [`POLL`].
    pub(crate) fn system() -> Arc<Self> {
        Self::with(Box::new(system_load), POLL, IDLE)
    }

    fn with(load: Box<Load>, poll: Duration, idle: Duration) -> Arc<Self> {
        Arc::new(Self { load, poll, idle, state: Mutex::default(), entered: Condvar::new() })
    }

    /// Registers a build. Returns the workers it starts with, its lease and the budgets that
    /// follow as the load and the other builds change.
    pub(crate) fn enter(self: &Arc<Self>) -> (usize, Lease, Receiver<usize>) {
        let (tx, rx) = channel();
        let mut s = self.lock();
        let id = s.next;
        s.next += 1;
        s.builds.push(Build { id, tx, last: 0 });
        let start = self.rebalance(&mut s, Some(id));
        if !s.sampling && self.sample(&mut s) {
            s.sampling = true;
        }
        drop(s);
        self.entered.notify_all();
        let lease = Lease { id, sched: Arc::clone(self) };
        (start, lease, rx)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Gives every build its share of the current budget, and returns the share of `quiet`,
    /// which is told nothing: it is the build just entering and learns it from the return.
    fn rebalance(&self, s: &mut State, quiet: Option<u64>) -> usize {
        let load = (self.load)().map(|l| l.saturating_sub(s.own_percent()));
        let each = share(tier(load), s.builds.len());
        for b in &mut s.builds {
            if b.last != each || Some(b.id) == quiet {
                b.last = each;
                // A build whose receiver is gone has ended: its lease removes it.
                if Some(b.id) != quiet && b.tx.send(each).is_err() {
                    tracing::debug!("index build {} ended before its budget", b.id);
                }
            }
        }
        each
    }

    fn sample(self: &Arc<Self>, _s: &mut State) -> bool {
        let me = Arc::clone(self);
        match std::thread::Builder::new().name("index-load".into()).spawn(move || me.sampler()) {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("index load sampler not started: {e}");
                false
            }
        }
    }

    /// Reads the load every poll while a build runs, returns once none did for the idle window.
    fn sampler(&self) {
        let mut s = self.lock();
        loop {
            if s.builds.is_empty() {
                let (g, out) = self
                    .entered
                    .wait_timeout_while(s, self.idle, |s| s.builds.is_empty())
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                s = g;
                if out.timed_out() && s.builds.is_empty() {
                    s.sampling = false;
                    return;
                }
                continue;
            }
            self.rebalance(&mut s, None);
            // Woken early only by a build entering, which has just rebalanced: wait the
            // rest of the poll like any other round.
            let (g, _) = self
                .entered
                .wait_timeout(s, self.poll)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            s = g;
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut s = self.sched.lock();
        s.builds.retain(|b| b.id != self.id);
        self.sched.rebalance(&mut s, None);
        drop(s);
        // The sampler waits out a whole poll: wake it to see the last build gone and start its
        // idle countdown now.
        self.sched.entered.notify_all();
    }
}

/// Load of the last minute in percent of the cores: `/proc/loadavg`, else the `vm.loadavg`
/// sysctl of the BSDs and macOS.
fn system_load() -> Option<u32> {
    if let Some(fixed) = std::env::var(LOAD_VAR).ok().and_then(|v| v.parse().ok()) {
        return Some(fixed);
    }
    let text = std::fs::read_to_string("/proc/loadavg").ok().or_else(|| {
        let out = std::process::Command::new("sysctl").args(["-n", "vm.loadavg"]).output().ok()?;
        String::from_utf8(out.stdout).ok()
    })?;
    let one: f64 = text.trim_start_matches(['{', ' ']).split_whitespace().next()?.parse().ok()?;
    let cores = std::thread::available_parallelism().map_or(1, usize::from) as f64;
    // Saturating float to int: a load is small and never negative.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a percent")]
    Some((one / cores * 100.0).max(0.0) as u32)
}

#[cfg(test)]
#[path = "sched_tests.rs"]
mod tests;
