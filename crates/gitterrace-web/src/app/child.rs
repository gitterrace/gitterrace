//! Index updates in a child process of this executable.
//!
//! A build reads and diffs the whole history on every core: 280 MB of heap at 100k commits.
//! Freed, it still leaves the allocator's metadata for every page it mapped and the stacks of
//! its finished workers, so a released server sat 4 to 5 MB higher at 100k commits than at 2k
//! on aarch64 Linux. In a child all of it goes with the child's exit, and the server only reads
//! the segment set it wrote, as it would after any other process extended it.
//!
//! The child is the same executable started again with the same arguments and
//! [`CHILD_VAR`] set, so a test harness that serves from its own executable gets one too.
//! Only a process whose `main` calls [`index_child`] starts children: elsewhere (the crate's
//! own tests, an embedder) the update runs in-process as before.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use gitterrace_git::Repo;
use gitterrace_index::{Index, Progress};

use crate::app::App;

/// Set in the child: the cache folder to update. The other settings ride in the vars below.
const CHILD_VAR: &str = "GITTERRACE_INDEX_CHILD";
const REV_VAR: &str = "GITTERRACE_INDEX_REV";
const GIT_VAR: &str = "GITTERRACE_INDEX_GIT_DIR";
const THREADS_VAR: &str = "GITTERRACE_INDEX_THREADS";
const MESSAGES_VAR: &str = "GITTERRACE_INDEX_MESSAGES";
/// Prefix of every line the child reports on stdout. A harness may print lines of its own.
const TAG: &str = "gitterrace-index";
/// Harness only: a folder. At its first progress report the child writes a file named by its
/// pid there, holding its cache folder, then waits forever, so a test can stop a repository
/// while its child is mid-build.
const HOLD_VAR: &str = "TEST_INDEX_CHILD_HOLD";

/// Whether this process may start index children, see [`index_child`].
static CHILDREN: AtomicBool = AtomicBool::new(false);

/// Call first in `main`. In an index child this runs the update and exits the process;
/// anywhere else it lets the servers of this process run their updates in children.
#[expect(
    clippy::print_stderr,
    reason = "the child's failure reaches the user through its stderr, which the server shares"
)]
pub fn index_child() {
    let Some(dir) = std::env::var_os(CHILD_VAR) else {
        CHILDREN.store(true, Ordering::Relaxed);
        return;
    };
    // A server that dies leaves this child with a closed stdin: stop with it, never keep
    // building for nobody.
    std::thread::spawn(|| {
        drop(io::copy(&mut io::stdin().lock(), &mut io::sink()));
        std::process::exit(3);
    });
    let code = match run(&PathBuf::from(dir)) {
        Ok(()) => 0,
        Err(e) => {
            let kind = if gitterrace_index::feed::SourceError::of(&e).is_some() {
                "unreadable"
            } else {
                "failed"
            };
            report(&format!("{kind} {}", one_line(&e.to_string())));
            eprintln!("gitterrace: index update failed: {e}");
            1
        }
    };
    std::process::exit(code);
}

/// The child's work: one [`Index::update`], its progress and steps reported on stdout.
fn run(dir: &std::path::Path) -> io::Result<()> {
    let var = |k: &str| std::env::var(k).map_err(|e| io::Error::other(format!("{k}: {e}")));
    let threads = var(THREADS_VAR)?.parse().map_err(io::Error::other)?;
    let repo = Repo::discover(&PathBuf::from(var(GIT_VAR)?))
        .map_err(io::Error::other)?
        .with_workers(threads);
    let index = Index::open_dir(dir.to_path_buf())?;
    index.set_message_index(var(MESSAGES_VAR)? == "1");
    // The merge would die half done with this process if it ran on a thread of its own.
    index.set_merge_inline(true);
    let hold = std::env::var_os(HOLD_VAR).map(PathBuf::from);
    index.update(&repo, &var(REV_VAR)?, &|p| {
        report(&format!("progress {} {}", p.done, p.total));
        if let Some(h) = &hold {
            hold_here(h, dir);
        }
    })?;
    let steps: Vec<String> = index
        .phases()
        .0
        .iter()
        .map(|p| format!("{}={}", p.name, p.secs))
        .collect();
    report(&format!("phases {}", steps.join(" ")));
    Ok(())
}

/// See [`HOLD_VAR`]. The file appears whole: written atomically.
fn hold_here(folder: &std::path::Path, cache: &std::path::Path) -> ! {
    let name = std::process::id().to_string();
    let bytes = cache.as_os_str().as_encoded_bytes();
    if let Err(e) = gitterrace_git::write_atomic(folder, &name, bytes, 0o600) {
        report(&format!("failed hold file: {}", one_line(&e.to_string())));
    }
    loop {
        std::thread::park();
    }
}

/// One tagged line to the server. A server gone away is no reason to stop here: the closed
/// stdin ends this process.
fn report(line: &str) {
    let mut out = io::stdout().lock();
    drop(writeln!(out, "{TAG} {line}").and_then(|()| out.flush()));
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Brings `app`'s index up to `rev`: in a child when this process starts children (see
/// [`index_child`]), else in-process. Progress and steps reach `app` as from an in-process
/// update, and an unreadable cache fails with the error [`SourceError::of`] finds.
///
/// [`SourceError::of`]: gitterrace_index::feed::SourceError::of
pub(crate) fn update(app: &App, rev: &str) -> io::Result<()> {
    if !CHILDREN.load(Ordering::Relaxed) {
        return app.index.update(&app.repo, rev, &|p| app.set_progress(p));
    }
    let cfg = app.cfg();
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.args(std::env::args_os().skip(1))
        .env(CHILD_VAR, app.index.dir())
        .env(REV_VAR, rev)
        .env(GIT_VAR, app.repo.git_dir())
        .env(THREADS_VAR, cfg.index.threads.to_string())
        .env(
            MESSAGES_VAR,
            if app.index.message_index() { "1" } else { "0" },
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let out = {
        // Spawned under the lock `App::halt` takes: a child is either seen by it or never
        // started.
        let mut slot = app.index_child.lock().map_err(|_| halted())?;
        if app.index.halted() {
            return Err(halted());
        }
        let mut child = cmd.spawn()?;
        // The stdin stays with the handle until the child is reaped: its closing is what
        // stops a child whose server died.
        let out = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no child stdout"))?;
        *slot = Some(child);
        out
    };
    let mut failure = None;
    for line in io::BufReader::new(out).lines() {
        let line = line?;
        let Some(rest) = line.strip_prefix(TAG).and_then(|l| l.strip_prefix(' ')) else {
            continue;
        };
        let (kind, body) = rest.split_once(' ').unwrap_or((rest, ""));
        match kind {
            "progress" => {
                let mut n = body.split(' ').filter_map(|v| v.parse().ok());
                if let (Some(done), Some(total)) = (n.next(), n.next()) {
                    app.set_progress(Progress { done, total });
                }
            }
            "phases" => app.index.adopt_phases(&steps(body)),
            "unreadable" => {
                failure = Some(io::Error::other(gitterrace_index::feed::SourceError(
                    body.to_owned(),
                )));
            }
            "failed" => failure = Some(io::Error::other(body.to_owned())),
            _ => {}
        }
    }
    // Gone from the slot: `App::halt` took it, killed and reaped it.
    let child = app.index_child.lock().map_err(|_| halted())?.take();
    let Some(mut child) = child else {
        return Err(halted());
    };
    let status = child.wait()?;
    if let Some(e) = failure {
        return Err(e);
    }
    if !status.success() {
        return Err(io::Error::other(format!("index child exited: {status}")));
    }
    Ok(())
}

fn halted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "index update halted")
}

impl App {
    fn set_progress(&self, p: Progress) {
        if let Ok(mut g) = self.progress.lock() {
            *g = p;
        }
    }

    /// Stops every index write for this repository, for one taken off the server: the
    /// in-process update returns at its next record, and the index child is killed and
    /// reaped before this returns, so nothing writes into the cache afterwards. Both leave
    /// the cache at its last whole set: each write step swaps its files in by one manifest
    /// rename.
    pub(crate) fn halt(&self) {
        self.index.halt();
        let mut g = match self.index_child.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if let Some(mut c) = g.take() {
            // Kill fails only for a child already gone; the wait reaps it either way.
            if let Err(e) = c.kill() {
                tracing::debug!("{}: index child already ended: {e}", self.name);
            }
            if let Err(e) = c.wait() {
                tracing::warn!("{}: reaping the index child: {e}", self.name);
            }
        }
    }
}

/// `name=secs` pairs as the child printed them.
fn steps(body: &str) -> Vec<(String, f64)> {
    body.split(' ')
        .filter_map(|s| {
            let (n, v) = s.split_once('=')?;
            Some((n.to_owned(), v.parse().ok()?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_read_back_as_the_child_printed_them() {
        assert_eq!(
            steps("rev_list=0.5 held=0 bad stream=x"),
            [("rev_list".to_owned(), 0.5), ("held".to_owned(), 0.0)]
        );
    }
}
