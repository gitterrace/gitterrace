//! Test support shared by every crate, so a helper is written once and cannot drift.
//!
//! Dev-dependency only: nothing here ships. `tests/conventions.rs` refuses a second copy of
//! the git helper or of the default address anywhere else in the workspace.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

pub mod a11y;
pub mod css;
mod fixture;
pub mod matrix;
pub mod oracle;
mod renames;
pub mod scan;
pub mod synthetic;
pub mod wait;
mod web;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub use fixture::{Adversarial, FAR_FUTURE, Fixture};
pub use gitterrace_config::DEFAULT_ADDR;
pub use renames::{MergeWithAuthor, RenameChain};
pub use web::{base, duplicate_ids, get, page, repeated_header_destinations};

/// The port of [`DEFAULT_ADDR`], for tests that build a `Host` or `Origin` from it.
#[must_use]
pub fn port() -> &'static str {
    DEFAULT_ADDR.rsplit(':').next().expect("addr has a port")
}

/// `http://` plus [`DEFAULT_ADDR`]: the base URL a default server hands out.
#[must_use]
pub fn base_url() -> String {
    format!("http://{DEFAULT_ADDR}")
}

/// `host` with the default port appended, as a `Host` header carries it.
#[must_use]
pub fn on_port(host: &str) -> String {
    format!("{host}:{}", port())
}

/// A global git config holding the identity and switches every fixture needs. It sits below a
/// repository's own config, so a test that sets `user.name` still wins. Automatic maintenance is
/// off: a detached `git maintenance` left a lock file in a fixture after the test had compared
/// its tree, on a macOS runner only.
fn global_config() -> &'static Path {
    static CONFIG: OnceLock<std::path::PathBuf> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let path = std::env::temp_dir().join(format!(
            "gitterrace-testkit-{}.gitconfig",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "[user]\n\tname = T\n\temail = t@e.x\n[commit]\n\tgpgsign = false\n[tag]\n\tgpgsign = \
            false\n[init]\n\tdefaultBranch = main\n[maintenance]\n\tauto = false\n[gc]\n\t\
            auto = 0\n",
        )
        .expect("write git config");
        path
    })
}

fn try_run(dir: &Path, date: Option<&str>, args: &[&str]) -> Result<String, String> {
    try_run_with(dir, date, args, None)
}

fn try_run_with(
    dir: &Path,
    date: Option<&str>,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", global_config())
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(d) = date {
        cmd.env("GIT_AUTHOR_DATE", d).env("GIT_COMMITTER_DATE", d);
    }
    let out = match stdin {
        None => cmd.output().expect("run git"),
        Some(input) => {
            let mut child = cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("run git");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(input)
                .expect("write stdin");
            child.wait_with_output().expect("wait git")
        }
    };
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

fn run(dir: &Path, date: Option<&str>, args: &[&str]) -> String {
    try_run(dir, date, args).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

/// Like [`git`], but a failing command is a value: stderr comes back as the error.
pub fn try_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    try_run(dir, None, args)
}

/// Runs `git args` in `dir`, panics with git's stderr on failure, returns trimmed stdout.
pub fn git(dir: &Path, args: &[&str]) -> String {
    run(dir, None, args)
}

/// [`git`] with author and committer date pinned to `date` (RFC 3339).
pub fn git_at(dir: &Path, date: &str, args: &[&str]) -> String {
    run(dir, Some(date), args)
}

/// [`git`] with `input` on stdin, for plumbing such as `hash-object --stdin` and `mktree`.
pub fn git_stdin(dir: &Path, args: &[&str], input: &[u8]) -> String {
    try_run_with(dir, None, args, Some(input)).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

/// [`git_stdin`] with the dates of [`git_at`] when `date` is set.
pub fn git_stdin_at(dir: &Path, date: Option<&str>, args: &[&str], input: &[u8]) -> String {
    try_run_with(dir, date, args, Some(input)).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}
