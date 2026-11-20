//! Wall time, CPU time and worker busy share of each step of an index update, kept for the
//! last update so the progress endpoint and the bench can name the slowest step and say
//! whether it was short of cores or waiting. Always on: a clock read per step, plus one per
//! folded record for the fold share of the stream.
//!
//! CPU time is the whole process's, so it is only meaningful where the update runs alone:
//! the index child. In-process it also counts the server's requests.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use gitterrace_git::WorkerTime;

/// Every step name an update records, so steps read back from another process keep a
/// static name.
const NAMES: [&str; 8] =
    ["rev_list", "held", "stream", "stream.fold", "append", "finish", "messages", "tip"];

/// One step of the last update.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PhaseTime {
    pub name: &'static str,
    /// Wall seconds.
    pub secs: f64,
    /// CPU seconds of the updating process in the step, `null` for a share measured inside
    /// another step (`stream.fold`) or where the system cannot say.
    pub cpu_secs: Option<f64>,
    /// The part of the parallel workers' lives in the step spent working, 0 to 1, `null` in a
    /// step that ran none. Low means the workers waited on the reorder window.
    pub busy: Option<f64>,
}

impl PhaseTime {
    /// One step as the index child prints it: `name=secs,cpu,busy`, `-` for none.
    pub fn encode(&self) -> String {
        let opt = |v: Option<f64>| v.map_or_else(|| "-".to_owned(), |v| v.to_string());
        format!("{}={},{},{}", self.name, self.secs, opt(self.cpu_secs), opt(self.busy))
    }

    /// The steps of a line [`PhaseTime::encode`] wrote, space separated. A step with an
    /// unknown name or a malformed time is left out.
    pub fn decode(line: &str) -> Vec<Self> {
        line.split(' ').filter_map(decode_one).collect()
    }
}

/// The steps of the update running now or last finished, and the one running.
#[derive(Debug, Default)]
pub(crate) struct Phases {
    inner: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    done: Vec<PhaseTime>,
    now: Option<Running>,
}

#[derive(Debug, Clone, Copy)]
struct Running {
    name: &'static str,
    wall: Instant,
    cpu: Option<Duration>,
    workers: WorkerTime,
}

impl Running {
    fn start(name: &'static str) -> Self {
        Self { name, wall: Instant::now(), cpu: process_cpu(), workers: WorkerTime::now() }
    }

    fn measure(self) -> PhaseTime {
        PhaseTime {
            name: self.name,
            secs: self.wall.elapsed().as_secs_f64(),
            cpu_secs: self.cpu.zip(process_cpu()).map(|(a, b)| b.saturating_sub(a).as_secs_f64()),
            busy: WorkerTime::now().since(self.workers).busy_share(),
        }
    }
}

impl Phases {
    /// Forgets the last update's steps: a new one starts.
    pub(crate) fn reset(&self) {
        if let Ok(mut s) = self.inner.lock() {
            *s = State::default();
        }
    }

    /// Ends the running step, if any, and starts `name`.
    pub(crate) fn start(&self, name: &'static str) {
        if let Ok(mut s) = self.inner.lock() {
            close(&mut s);
            s.now = Some(Running::start(name));
        }
    }

    /// Ends the running step.
    pub(crate) fn end(&self) {
        if let Ok(mut s) = self.inner.lock() {
            close(&mut s);
        }
    }

    /// Records a step measured elsewhere (the fold share inside the stream).
    pub(crate) fn add(&self, name: &'static str, took: Duration) {
        if let Ok(mut s) = self.inner.lock() {
            s.done.push(PhaseTime { name, secs: took.as_secs_f64(), cpu_secs: None, busy: None });
        }
    }

    /// Replaces the steps with those of an update run elsewhere.
    pub(crate) fn adopt(&self, steps: Vec<PhaseTime>) {
        if let Ok(mut s) = self.inner.lock() {
            *s = State { done: steps, now: None };
        }
    }

    /// The finished steps, and the running one with its time so far.
    pub(crate) fn snapshot(&self) -> (Vec<PhaseTime>, Option<&'static str>) {
        let Ok(s) = self.inner.lock() else {
            return (Vec::new(), None);
        };
        let mut done = s.done.clone();
        if let Some(r) = s.now {
            done.push(r.measure());
        }
        (done, s.now.map(|r| r.name))
    }
}

fn close(s: &mut State) {
    if let Some(r) = s.now.take() {
        s.done.push(r.measure());
    }
}

fn decode_one(field: &str) -> Option<PhaseTime> {
    let (n, v) = field.split_once('=')?;
    let name = NAMES.iter().find(|k| **k == n)?;
    let mut parts = v.split(',');
    let secs = parts.next()?.parse().ok()?;
    let mut opt = || parts.next().and_then(|p| p.parse().ok());
    let cpu_secs = opt();
    let busy = opt();
    Some(PhaseTime { name, secs, cpu_secs, busy })
}

/// CPU time of this process so far, user plus system, over all its threads.
#[cfg(unix)]
fn process_cpu() -> Option<Duration> {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
    Duration::try_from(t).ok()
}

#[cfg(not(unix))]
const fn process_cpu() -> Option<Duration> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_read_back_as_the_child_printed_them() {
        let steps = [
            PhaseTime { name: "stream", secs: 2.5, cpu_secs: Some(18.0), busy: Some(0.75) },
            PhaseTime { name: "stream.fold", secs: 0.5, cpu_secs: None, busy: None },
        ];
        let line: Vec<String> = steps.iter().map(PhaseTime::encode).collect();
        assert_eq!(PhaseTime::decode(&line.join(" ")), steps);
        // Unknown names and malformed times are dropped, a missing CPU or busy reads as none.
        let odd = PhaseTime::decode("bogus=1,2,3 held=x rev_list=0.5");
        assert_eq!(odd, [PhaseTime { name: "rev_list", secs: 0.5, cpu_secs: None, busy: None }]);
    }

    /// CPU spent inside a step is charged to it. Spins until the process clock moves, so a
    /// clock that never moves, or a step that never reads it, never finishes or reads 0.
    #[cfg(unix)]
    #[test]
    fn cpu_spent_in_a_step_is_charged_to_it() {
        let p = Phases::default();
        p.start("held");
        let from = process_cpu();
        assert!(from.is_some(), "no process clock");
        let mut x = 0_u64;
        while process_cpu() <= from {
            x = std::hint::black_box(x.wrapping_add(1));
        }
        p.end();
        let (done, _) = p.snapshot();
        assert!(done[0].cpu_secs.is_some_and(|c| c > 0.0), "{done:?}");
    }
}
