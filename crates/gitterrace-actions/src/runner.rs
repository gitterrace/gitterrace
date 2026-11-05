//! Spawns a configured action.
//!
//! The payload reaches the script as data only (stdin JSON and
//! `GITTERRACE_*` env), the environment is scrubbed, the whole process group is killed on
//! timeout, and concurrency and rate limits bound what a runaway client can start.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gitterrace_git::percent_encode_segment;

use crate::config::{Action, Cwd, Piece, REPO_FIELDS, Stdin, field_parts, pieces};
use crate::payload::Payload;

/// The first output line returned to the browser. Full output never leaves the server.
/// Not configurable: it is the size of a toast, part of the page contract.
const MAX_FIRST_LINE: usize = 200;
/// Between SIGTERM and SIGKILL on timeout.
const KILL_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);
/// Inherited variables a script may see; everything else (tokens, agent sockets) is dropped
/// unless the action names it in `env`.
const KEEP_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "TMPDIR",
    #[cfg(windows)]
    "SystemRoot",
    #[cfg(windows)]
    "USERPROFILE",
    #[cfg(windows)]
    "TEMP",
    #[cfg(windows)]
    "PATHEXT",
];
/// One expanded placeholder value. Larger values belong on stdin, not in argv.
/// Not configurable: argv bounds keep a spawn under the OS `ARG_MAX`.
const MAX_VALUE_BYTES: usize = 64 * 1024;
/// The whole expanded argv, well under every OS's `ARG_MAX` with the environment beside it.
/// Not configurable: an OS bound (`ARG_MAX`), not a preference.
const MAX_ARGV_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_running: usize,
    pub per_minute: usize,
    /// Output kept per stream; the rest is read and dropped so the child never blocks on a
    /// pipe (`limits.action_output_kb`).
    pub output_bytes: usize,
}

impl Default for Limits {
    /// 4 running process-wide (one per action id), 30 starts a minute and 64 KiB of output kept
    /// per stream.
    fn default() -> Self {
        Self {
            max_running: 4,
            per_minute: 30,
            output_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub ok: bool,
    /// `None` when killed by a signal (timeout included).
    pub exit: Option<i32>,
    pub timed_out: bool,
    /// First stdout line on success, first stderr line on failure; capped.
    pub first_line: String,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The 429s: a run of this action is still going, 4 are running, or the minute budget is spent.
    #[error("still running")]
    Busy,
    #[error("too many runs, try again in a minute")]
    RateLimited,
    #[error("spawn {0}: {1}")]
    Spawn(String, std::io::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("payload: {0}")]
    Json(#[from] serde_json::Error),
    /// The selection cannot be passed as arguments; shown to the user as is.
    #[error("{0}")]
    Refused(String),
}

#[derive(Default)]
struct State {
    running: HashSet<String>,
    starts: VecDeque<Instant>,
}

/// Shared across requests; cheap to clone.
#[derive(Clone)]
pub struct Runner {
    limits: Limits,
    state: Arc<Mutex<State>>,
}

/// Releases the action's slot however the run ends.
struct Slot {
    state: Arc<Mutex<State>>,
    id: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.running.remove(&self.id);
        }
    }
}

impl Runner {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            state: Arc::default(),
        }
    }

    /// Runs `action` with `payload`, blocking until it exits or times out. `repo_root` is the
    /// cwd for `cwd = "repo"`. Call from a blocking thread.
    pub fn run(
        &self,
        action: &Action,
        payload: &Payload,
        repo_root: &Path,
    ) -> Result<Outcome, RunError> {
        let _slot = self.acquire(&action.id)?;
        let mut payload = payload.clone();
        payload.vars.clone_from(&action.vars);
        let argv = spawn_argv(action, &payload)?;
        let stdin = match action.stdin {
            Stdin::Json => Some(serde_json::to_vec(&payload)?),
            Stdin::Markdown => Some(payload.markdown.clone().into_bytes()),
            Stdin::None => None,
        };
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .env_clear()
            .current_dir(match &action.cwd {
                Cwd::Repo => repo_root.to_path_buf(),
                Cwd::Home => std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                    .map_or_else(|| std::path::MAIN_SEPARATOR_STR.into(), Into::into),
                Cwd::Path(p) => p.clone(),
            })
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        new_group(&mut cmd);
        for (k, v) in std::env::vars_os() {
            let name = k.to_string_lossy();
            if KEEP_ENV.contains(&name.as_ref())
                || name.starts_with("LC_")
                || action.env.iter().any(|e| *e == name)
            {
                cmd.env(&k, v);
            }
        }
        // Vars first, payload last: even a colliding name could not replace a payload value.
        cmd.envs(payload.var_env());
        cmd.envs(payload.env());
        let child = cmd
            .spawn()
            .map_err(|e| RunError::Spawn(action.argv[0].clone(), e))?;
        Ok(supervise(
            child,
            stdin,
            action.timeout,
            self.limits.output_bytes,
        ))
    }

    fn acquire(&self, id: &str) -> Result<Slot, RunError> {
        let mut s = self.state.lock().map_err(|_| RunError::Busy)?;
        if s.running.contains(id) || s.running.len() >= self.limits.max_running {
            return Err(RunError::Busy);
        }
        let now = Instant::now();
        while s
            .starts
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
        {
            s.starts.pop_front();
        }
        if s.starts.len() >= self.limits.per_minute {
            return Err(RunError::RateLimited);
        }
        s.starts.push_back(now);
        s.running.insert(id.to_string());
        Ok(Slot {
            state: Arc::clone(&self.state),
            id: id.to_string(),
        })
    }
}

/// The argv `run` spawns for `action`. A shell script is passed verbatim: its braces are shell
/// text, and the loader has already refused placeholders in it.
pub fn spawn_argv(action: &Action, payload: &Payload) -> Result<Vec<String>, RunError> {
    if action.shell {
        Ok(action.argv.clone())
    } else {
        expand(&action.argv, payload)
    }
}

/// Fills each element of the template with the payload's values. Each element stays one
/// argument; no shell sees it. Refuses rather than truncates: a cut title is a wrong title.
pub fn expand(template: &[String], payload: &Payload) -> Result<Vec<String>, RunError> {
    let mut out = Vec::with_capacity(template.len());
    let mut total = 0;
    let mut after_dashdash = false;
    for (i, el) in template.iter().enumerate() {
        if i == 0 {
            out.push(el.clone());
            continue;
        }
        let ps = pieces(el).map_err(|e| RunError::Refused(e.to_string()))?;
        let mut arg = String::new();
        for p in &ps {
            match p {
                Piece::Text(t) => arg.push_str(t),
                Piece::Field(name) => {
                    let (base, modifier) = field_parts(name);
                    let raw = payload.placeholder(base).ok_or_else(|| {
                        RunError::Refused(format!("unknown placeholder {{{name}}}"))
                    })?;
                    // The NUL check and the caps see the encoded value, the one passed on.
                    let v = match modifier {
                        None => raw,
                        Some("url") => percent_encode_segment(&raw),
                        Some(m) => {
                            return Err(RunError::Refused(format!(
                                "{{{name}}}: unknown modifier :{m}"
                            )));
                        }
                    };
                    if v.contains('\0') {
                        return Err(RunError::Refused(format!("{{{name}}} contains a NUL byte")));
                    }
                    if v.len() > MAX_VALUE_BYTES {
                        return Err(RunError::Refused(format!(
                            "{{{name}}} is {} KiB, over the 64 KiB argument cap. Use stdin",
                            v.len() / 1024
                        )));
                    }
                    arg.push_str(&v);
                }
            }
        }
        // The loader lets a repository value open an element only behind `--` or right after
        // a literal option; in the second case a leading `-` would still read as an option.
        let repo_first = matches!(ps.first(),
            Some(Piece::Field(n)) if REPO_FIELDS.contains(&field_parts(n).0));
        if !after_dashdash && repo_first && arg.starts_with('-') {
            return Err(RunError::Refused(format!(
                "command[{i}] would start with '-' and read as an option"
            )));
        }
        after_dashdash |= el == "--";
        total += arg.len() + 1;
        if total > MAX_ARGV_BYTES {
            return Err(RunError::Refused(
                "the arguments are over 256 KiB together. Use stdin".into(),
            ));
        }
        out.push(arg);
    }
    Ok(out)
}

fn supervise(mut child: Child, stdin: Option<Vec<u8>>, timeout: Duration, keep: usize) -> Outcome {
    let pgid = group_of(&child);
    // Writing on its own thread: a script that never reads stdin must not stall us.
    if let (Some(mut pipe), Some(stdin)) = (child.stdin.take(), stdin) {
        thread::spawn(move || {
            if pipe.write_all(&stdin).is_err() {
                // The script closed stdin early; that is its choice.
            }
        });
    }
    let out = child.stdout.take().map(|r| capture(r, keep));
    let err = child.stderr.take().map(|r| capture(r, keep));

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                timed_out = true;
                signal_group(pgid, Sig::Term);
                let grace = Instant::now() + KILL_GRACE;
                while Instant::now() < grace && matches!(child.try_wait(), Ok(None)) {
                    thread::sleep(POLL);
                }
                signal_group(pgid, Sig::Kill);
                break child.wait().ok();
            }
            Err(_) => break None,
        }
    };
    // Leftover children (backgrounded by the script) hold the pipes open and outlive the
    // click; the group dies with the run.
    // Windows has no group left to name once the leader exited, so only a timeout kills there.
    #[cfg(unix)]
    signal_group(pgid, Sig::Kill);

    let collect = |rx: Option<mpsc::Receiver<Vec<u8>>>| {
        rx.and_then(|r| r.recv_timeout(KILL_GRACE).ok())
            .unwrap_or_default()
    };
    let (stdout, stderr) = (collect(out), collect(err));
    let exit = status.and_then(|s| s.code());
    let ok = !timed_out && exit == Some(0);
    let first_line = if timed_out {
        format!("timed out after {}s", timeout.as_secs_f32())
    } else {
        let pick = if ok || stderr.is_empty() {
            &stdout
        } else {
            &stderr
        };
        first_line(pick).unwrap_or_else(|| {
            status
                .and_then(exit_signal)
                .map_or_else(String::new, |sig| format!("killed by signal {sig}"))
        })
    };
    tracing::debug!(target: "gitterrace::actions", ok, ?exit, stdout = %String::from_utf8_lossy(&stdout), stderr = %String::from_utf8_lossy(&stderr), "action finished");
    Outcome {
        ok,
        exit,
        timed_out,
        first_line,
        stdout,
        stderr,
    }
}

#[derive(Clone, Copy)]
enum Sig {
    Term,
    Kill,
}

#[cfg(unix)]
type Group = rustix::process::Pid;
#[cfg(windows)]
type Group = u32;

#[cfg(unix)]
fn new_group(cmd: &mut Command) {
    cmd.process_group(0);
}

/// `CREATE_NEW_PROCESS_GROUP`: the child gets its own group so a console Ctrl event meant
/// for the server never reaches it, and `taskkill /T` can walk its tree.
#[cfg(windows)]
fn new_group(cmd: &mut Command) {
    cmd.creation_flags(0x0000_0200);
}

#[cfg(unix)]
fn group_of(child: &Child) -> Group {
    rustix::process::Pid::from_child(child)
}

#[cfg(windows)]
fn group_of(child: &Child) -> Group {
    child.id()
}

#[cfg(unix)]
fn exit_signal(s: std::process::ExitStatus) -> Option<i32> {
    s.signal()
}

#[cfg(windows)]
fn exit_signal(_: std::process::ExitStatus) -> Option<i32> {
    None
}

#[cfg(unix)]
fn signal_group(pgid: Group, sig: Sig) {
    use rustix::process::{Signal, kill_process_group};
    let sig = match sig {
        Sig::Term => Signal::TERM,
        Sig::Kill => Signal::KILL,
    };
    if kill_process_group(pgid, sig).is_err() {
        // ESRCH: the group is already gone.
    }
}

/// `taskkill /T` ends the whole tree; `/F` is the forceful kill after the grace period.
#[cfg(windows)]
fn signal_group(pid: Group, sig: Sig) {
    let mut cmd = Command::new("taskkill");
    cmd.arg("/T");
    if matches!(sig, Sig::Kill) {
        cmd.arg("/F");
    }
    cmd.arg("/PID")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if cmd.status().is_err() {
        // taskkill missing or the tree already gone.
    }
}

/// Reads a stream to EOF on a thread, keeping the first `keep` bytes.
fn capture(mut r: impl Read + Send + 'static, keep: usize) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 {
                break;
            }
            let room = keep.saturating_sub(kept.len());
            kept.extend_from_slice(&buf[..n.min(room)]);
        }
        if tx.send(kept).is_err() {
            // The supervisor gave up waiting; nothing to deliver to.
        }
    });
    rx
}

fn first_line(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line: String = line
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_FIRST_LINE)
        .collect();
    Some(line)
}

// The fixtures use /bin/sh, chmod modes and symlinks, so they run on unix only.
#[cfg(test)]
#[cfg(unix)]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "the tests spell action placeholders like {title}, which are not format strings"
)]
mod tests;
