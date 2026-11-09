//! Wall time of each step of an index update, kept for the last update so the progress
//! endpoint and the bench can name the slowest step. Always on: a clock read per step, plus
//! one per folded record for the fold share of the stream.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One step of the last update and its wall time in seconds.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PhaseTime {
    pub name: &'static str,
    pub secs: f64,
}

/// The steps of the update running now or last finished, and the one running.
#[derive(Debug, Default)]
pub(crate) struct Phases {
    inner: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    done: Vec<PhaseTime>,
    now: Option<(&'static str, Instant)>,
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
            s.now = Some((name, Instant::now()));
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
            s.done.push(PhaseTime {
                name,
                secs: took.as_secs_f64(),
            });
        }
    }

    /// The finished steps, and the running one with its time so far.
    pub(crate) fn snapshot(&self) -> (Vec<PhaseTime>, Option<&'static str>) {
        let Ok(s) = self.inner.lock() else {
            return (Vec::new(), None);
        };
        let mut done = s.done.clone();
        if let Some((name, t)) = s.now {
            done.push(PhaseTime {
                name,
                secs: t.elapsed().as_secs_f64(),
            });
        }
        (done, s.now.map(|(n, _)| n))
    }
}

fn close(s: &mut State) {
    if let Some((name, t)) = s.now.take() {
        s.done.push(PhaseTime {
            name,
            secs: t.elapsed().as_secs_f64(),
        });
    }
}
