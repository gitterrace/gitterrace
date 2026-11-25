//! The one way this process starts a child.
//!
//! Where the OS has no atomic close-on-exec pipe (macOS), std makes a pipe and only then marks
//! it close-on-exec. A child forked by another thread in between inherits the pipe end, and a
//! reader waiting for its EOF (a script's `cat`) then waits on that unrelated child, which may
//! live as long as the server. Every spawn in a server process takes `SPAWN` for the span
//! from pipe creation to exec, so no two overlap. The ast-grep rule `spawn-through-the-lock`
//! refuses a bare `.spawn()`, `.output()` or `.status()` elsewhere.

use std::io;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Mutex, PoisonError};

static SPAWN: Mutex<()> = Mutex::new(());

/// Starts `cmd`, alone: no other spawn in this process runs while its pipes are made.
///
/// # Errors
/// What `Command::spawn` reports.
pub fn spawn_locked(cmd: &mut Command) -> io::Result<Child> {
    let _one = SPAWN.lock().unwrap_or_else(PoisonError::into_inner);
    cmd.spawn()
}

/// Runs `cmd` to its end with its output captured and stdin closed. Only the start holds the
/// lock, never the run.
///
/// # Errors
/// What spawning or waiting reports.
pub fn output_locked(cmd: &mut Command) -> io::Result<Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    spawn_locked(cmd)?.wait_with_output()
}

/// Runs `cmd` to its end with inherited stdio. Only the start holds the lock.
///
/// # Errors
/// What spawning or waiting reports.
pub fn status_locked(cmd: &mut Command) -> io::Result<ExitStatus> {
    spawn_locked(cmd)?.wait()
}
