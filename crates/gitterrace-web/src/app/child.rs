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
/// The workers the child may run at the start, all of `index.threads` when unset; later
/// changes come as [`BUDGET_LINE`] lines on its stdin.
const BUDGET_VAR: &str = "GITTERRACE_INDEX_BUDGET";
/// A line on the child's stdin, `budget <n>`: from now on at most `n` index workers run.
const BUDGET_LINE: &str = "budget";
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
    let budget = gitterrace_git::Budget::new(
        std::env::var(BUDGET_VAR).ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX),
    );
    let lines = budget.clone();
    // A server that dies leaves this child with a closed stdin: stop with it, never keep
    // building for nobody. Until then each line it sends sets the worker budget.
    std::thread::spawn(move || {
        follow(io::stdin().lock(), &lines);
        std::process::exit(3);
    });
    let code = match run(&PathBuf::from(dir), budget) {
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
fn run(dir: &std::path::Path, budget: gitterrace_git::Budget) -> io::Result<()> {
    let var = |k: &str| std::env::var(k).map_err(|e| io::Error::other(format!("{k}: {e}")));
    let threads = var(THREADS_VAR)?.parse().map_err(io::Error::other)?;
    let repo = Repo::discover(&PathBuf::from(var(GIT_VAR)?))
        .map_err(io::Error::other)?
        .with_workers(threads)
        .with_budget(budget);
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
    let steps: Vec<String> =
        index.phases().0.iter().map(gitterrace_index::PhaseTime::encode).collect();
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

/// Applies every [`BUDGET_LINE`] line of `input` to `budget`, each acknowledged on stdout,
/// and returns when it closes or breaks. Other lines are ignored.
fn follow(input: impl BufRead, budget: &gitterrace_git::Budget) {
    for line in input.lines() {
        let Ok(line) = line else { return };
        if let Some(n) = parse_budget(&line) {
            budget.set(n);
            report(&format!("{BUDGET_LINE} {n}"));
        }
    }
}

/// The worker count of a [`BUDGET_LINE`] line, `None` for any other line.
fn parse_budget(line: &str) -> Option<usize> {
    line.strip_prefix(BUDGET_LINE)?.strip_prefix(' ')?.trim().parse().ok()
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
    // One child per repository at a time: a second spawn would replace the first in the slot,
    // and `App::halt` would then kill only the second while the first, its handle dropped,
    // exits on its closed stdin and stays a zombie nobody reaps.
    let _one = app.index_run.lock().map_err(|_| halted())?;
    let cfg = app.cfg();
    // The scheduler sets the workers the build starts with and sends the changes after.
    let (start, _lease, budgets) = app.env.sched.enter();
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.args(std::env::args_os().skip(1))
        .env(CHILD_VAR, app.index.dir())
        .env(REV_VAR, rev)
        .env(GIT_VAR, app.repo.git_dir())
        .env(THREADS_VAR, cfg.index.threads.to_string())
        .env(BUDGET_VAR, start.to_string())
        .env(MESSAGES_VAR, if app.index.message_index() { "1" } else { "0" })
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
        // The stdin goes to the thread that forwards budgets, which ends with the lease. It
        // closes with the child or with this process, which is what stops a child whose
        // server died.
        if let Some(stdin) = child.stdin.take() {
            forward(stdin, budgets);
        }
        let out = child.stdout.take().ok_or_else(|| io::Error::other("no child stdout"))?;
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
            "phases" => app.index.adopt_phases(gitterrace_index::PhaseTime::decode(body)),
            "unreadable" => {
                failure =
                    Some(io::Error::other(gitterrace_index::feed::SourceError(body.to_owned())));
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

/// Sends each budget of `budgets` to the child as a [`BUDGET_LINE`] line, until the build's
/// lease ends or the child's stdin breaks.
fn forward(mut stdin: std::process::ChildStdin, budgets: std::sync::mpsc::Receiver<usize>) {
    let spawned = std::thread::Builder::new().name("index-budget".into()).spawn(move || {
        for n in budgets {
            if writeln!(stdin, "{BUDGET_LINE} {n}").and_then(|()| stdin.flush()).is_err() {
                return;
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("index budget thread not started: {e}");
    }
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

#[cfg(test)]
mod tests {
    /// Each budget line sets the budget, in order, and anything else leaves it alone; the
    /// end of input returns, which the child turns into its exit. Catches a reader that stops
    /// at the first line, applies a malformed one, or keeps reading past the end.
    #[test]
    fn budget_lines_apply_in_order_and_the_end_of_input_returns() {
        let budget = gitterrace_git::Budget::new(8);
        let input = "budget 2\nbudget x\nhello\nbudgets 9\nbudget 5\n";
        super::follow(input.as_bytes(), &budget);
        assert_eq!(budget.get(), 5);
        assert_eq!(super::parse_budget("budget 3"), Some(3));
        assert_eq!(super::parse_budget("budget -1"), None);
    }
}
