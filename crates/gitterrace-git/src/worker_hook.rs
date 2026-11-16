//! A hook the binary installs to run around each parallel index worker.
//!
//! The binary runs jemalloc without thread caches, so an idle server's freed objects do not
//! pin pages. The index build allocates on every core at once and runs twice as slow without
//! them (git.git: 21.6 s against 10.8 s). Only the binary links jemalloc, so it hands this
//! crate a switch: on when a worker starts, off (which flushes the cache) when it ends.

use std::sync::OnceLock;

static HOOK: OnceLock<fn(bool)> = OnceLock::new();

/// Installs `hook`, called with `true` as an index worker thread starts and `false` before it
/// ends. The first install wins.
pub fn set_worker_hook(hook: fn(bool)) {
    // A second install keeps the first: one allocator per process.
    HOOK.get_or_init(|| hook);
}

/// Calls the hook with `on`, if one is installed.
pub(crate) fn worker(on: bool) {
    if let Some(h) = HOOK.get() {
        h(on);
    }
}

/// Runs the hook on now and off when dropped, so a worker that returns early still flushes.
pub(crate) struct WorkerGuard;

impl WorkerGuard {
    pub(crate) fn start() -> Self {
        worker(true);
        Self
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        worker(false);
    }
}
