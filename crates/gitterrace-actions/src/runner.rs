//! Spawns a configured action. The payload reaches the script as data only (stdin JSON and
//! `GITTERRACE_*` env), the environment is scrubbed, the whole process group is killed on
//! timeout, and concurrency and rate limits bound what a runaway client can start.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

use crate::config::{Action, Cwd};
use crate::payload::Payload;

/// Captured per stream; the rest is read and dropped so the child never blocks on a pipe.
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// The first output line returned to the browser. Full output never leaves the server.
const MAX_FIRST_LINE: usize = 200;
/// Between SIGTERM and SIGKILL on timeout.
const KILL_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);
/// Inherited variables a script may see; everything else (tokens, agent sockets) is dropped
/// unless the action names it in `env`.
const KEEP_ENV: &[&str] = &["PATH", "HOME", "USER", "LANG", "TMPDIR"];

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_running: usize,
    pub per_minute: usize,
}

impl Default for Limits {
    /// 4 running process-wide (one per action id) and 30 starts a minute.
    fn default() -> Self {
        Self {
            max_running: 4,
            per_minute: 30,
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
        let stdin = serde_json::to_vec(payload)?;
        let mut cmd = Command::new(&action.argv[0]);
        cmd.args(&action.argv[1..])
            .env_clear()
            .current_dir(match &action.cwd {
                Cwd::Repo => repo_root.to_path_buf(),
                Cwd::Home => std::env::var_os("HOME").map_or_else(|| "/".into(), Into::into),
                Cwd::Path(p) => p.clone(),
            })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        for (k, v) in std::env::vars_os() {
            let name = k.to_string_lossy();
            if KEEP_ENV.contains(&name.as_ref())
                || name.starts_with("LC_")
                || action.env.iter().any(|e| *e == name)
            {
                cmd.env(&k, v);
            }
        }
        cmd.envs(payload.env());
        let child = cmd
            .spawn()
            .map_err(|e| RunError::Spawn(action.argv[0].clone(), e))?;
        Ok(supervise(child, stdin, action.timeout))
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

fn supervise(mut child: Child, stdin: Vec<u8>, timeout: Duration) -> Outcome {
    let pgid = Pid::from_child(&child);
    // Writing on its own thread: a script that never reads stdin must not stall us.
    if let Some(mut pipe) = child.stdin.take() {
        thread::spawn(move || {
            if pipe.write_all(&stdin).is_err() {
                // The script closed stdin early; that is its choice.
            }
        });
    }
    let out = child.stdout.take().map(capture);
    let err = child.stderr.take().map(capture);

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                timed_out = true;
                signal_group(pgid, Signal::TERM);
                let grace = Instant::now() + KILL_GRACE;
                while Instant::now() < grace && matches!(child.try_wait(), Ok(None)) {
                    thread::sleep(POLL);
                }
                signal_group(pgid, Signal::KILL);
                break child.wait().ok();
            }
            Err(_) => break None,
        }
    };
    // Leftover children (backgrounded by the script) hold the pipes open and outlive the
    // click; the group dies with the run.
    signal_group(pgid, Signal::KILL);

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
                .and_then(|s| s.signal())
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

fn signal_group(pgid: Pid, sig: Signal) {
    if kill_process_group(pgid, sig).is_err() {
        // ESRCH: the group is already gone.
    }
}

/// Reads a stream to EOF on a thread, keeping the first [`MAX_OUTPUT_BYTES`].
fn capture(mut r: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 {
                break;
            }
            let room = MAX_OUTPUT_BYTES.saturating_sub(kept.len());
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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use gitterrace_git::Repo;

    use crate::config::{Action, Cwd, Kind, Output};
    use crate::payload::{Lines, Locator, Payload};
    use crate::runner::{Limits, RunError, Runner};

    fn shell(id: &str, script: &str, timeout: Duration) -> Action {
        Action {
            id: id.into(),
            label: id.into(),
            kinds: vec![Kind::Line, Kind::Commit],
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            cwd: Cwd::Repo,
            timeout,
            output: Output::Toast,
            key: None,
            env: vec![],
        }
    }

    fn payload(repo: &Repo, kind: Kind) -> Payload {
        let lines = (kind == Kind::Line).then_some(Lines { start: 1, end: 2 });
        let loc = Locator {
            repo: "r".into(),
            rev: "main".into(),
            path: lines.map(|_| "src/a.rs".into()),
            lines,
            sha: None,
            query: None,
        };
        Payload::build(repo, "t", kind, &loc, "http://127.0.0.1:4242").expect("payload")
    }

    fn fixture() -> (tempfile::TempDir, Repo) {
        let (t, _) = crate::payload::tests::hostile_repo();
        let r = Repo::discover(t.path()).expect("open");
        (t, r)
    }

    #[test]
    fn script_gets_exact_stdin_and_only_allowed_env() {
        let (t, repo) = fixture();
        let out = t.path().join("seen");
        let script = format!(
            "cat > '{0}.json'; env > '{0}.env'; pwd > '{0}.pwd'; echo first; echo second",
            out.display()
        );
        let p = payload(&repo, Kind::Line);
        let r = Runner::new(Limits::default())
            .run(
                &shell("a", &script, Duration::from_secs(5)),
                &p,
                &repo.root(),
            )
            .expect("run");
        assert!(r.ok, "{r:?}");
        assert_eq!(r.first_line, "first");
        let stdin = std::fs::read(t.path().join("seen.json")).expect("stdin");
        assert_eq!(stdin, serde_json::to_vec(&p).expect("json"));
        let env = std::fs::read_to_string(t.path().join("seen.env")).expect("env");
        let allowed = [
            "PATH", "HOME", "USER", "LANG", "TMPDIR", "PWD", "SHLVL", "OLDPWD", "_",
        ];
        for line in env.lines() {
            let name = line.split('=').next().unwrap_or_default();
            assert!(
                allowed.contains(&name)
                    || name.starts_with("LC_")
                    || name.starts_with("GITTERRACE_"),
                "leaked {name}"
            );
        }
        assert!(env.contains("GITTERRACE_PATH=src/a.rs"));
        assert_eq!(
            std::fs::read_to_string(t.path().join("seen.pwd"))
                .expect("pwd")
                .trim(),
            repo.root().to_string_lossy()
        );
    }

    #[test]
    fn repo_content_never_becomes_shell() {
        let (t, repo) = fixture();
        let p = payload(&repo, Kind::Commit);
        assert!(p.title.contains("$(touch pwned)"));
        let r = Runner::new(Limits::default())
            .run(
                &shell(
                    "a",
                    "cat >/dev/null; echo \"$GITTERRACE_TITLE\"",
                    Duration::from_secs(5),
                ),
                &p,
                &repo.root(),
            )
            .expect("run");
        assert!(r.ok);
        assert!(
            r.first_line.contains("$(touch pwned)"),
            "printed literally: {}",
            r.first_line
        );
        assert!(!t.path().join("pwned").exists());
    }

    #[test]
    fn failure_reports_stderr_and_exit() {
        let (_t, repo) = fixture();
        let r = Runner::new(Limits::default())
            .run(
                &shell(
                    "a",
                    "echo out; echo boom >&2; exit 3",
                    Duration::from_secs(5),
                ),
                &payload(&repo, Kind::Commit),
                &repo.root(),
            )
            .expect("run");
        assert_eq!(
            (r.ok, r.exit, r.first_line.as_str()),
            (false, Some(3), "boom")
        );
    }

    #[test]
    fn timeout_kills_the_whole_group() {
        let (t, repo) = fixture();
        let pidfile = t.path().join("grandchild.pid");
        let script = format!(
            "sh -c 'echo $$ > \"{}\"; sleep 30' & sleep 30",
            pidfile.display()
        );
        let start = Instant::now();
        let r = Runner::new(Limits::default())
            .run(
                &shell("a", &script, Duration::from_millis(300)),
                &payload(&repo, Kind::Commit),
                &repo.root(),
            )
            .expect("run");
        assert!(r.timed_out && !r.ok);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .expect("pid")
            .trim()
            .parse()
            .expect("int");
        std::thread::sleep(Duration::from_millis(100));
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .expect("kill")
            .success();
        assert!(!alive, "grandchild {pid} survived");
    }

    #[test]
    fn output_is_capped() {
        let (_t, repo) = fixture();
        let r = Runner::new(Limits::default())
            .run(
                &shell(
                    "a",
                    "cat >/dev/null; head -c 300000 /dev/zero | tr '\\0' x",
                    Duration::from_secs(5),
                ),
                &payload(&repo, Kind::Commit),
                &repo.root(),
            )
            .expect("run");
        assert_eq!(r.stdout.len(), 64 * 1024);
        assert_eq!(r.first_line.len(), 200);
    }

    #[test]
    fn concurrency_and_rate_limits() {
        let (_t, repo) = fixture();
        let repo = std::sync::Arc::new(repo);
        let runner = Runner::new(Limits::default());
        let mut handles = Vec::new();
        for i in 0..4 {
            let (runner, repo) = (runner.clone(), repo.clone());
            handles.push(std::thread::spawn(move || {
                let p = payload(&repo, Kind::Commit);
                runner.run(
                    &shell(&format!("a{i}"), "sleep 1", Duration::from_secs(5)),
                    &p,
                    Path::new("/"),
                )
            }));
        }
        std::thread::sleep(Duration::from_millis(300));
        let p = payload(&repo, Kind::Commit);
        assert!(
            matches!(
                runner.run(
                    &shell("a9", "true", Duration::from_secs(5)),
                    &p,
                    Path::new("/")
                ),
                Err(RunError::Busy)
            ),
            "5th concurrent"
        );
        assert!(
            matches!(
                runner.run(
                    &shell("a0", "true", Duration::from_secs(5)),
                    &p,
                    Path::new("/")
                ),
                Err(RunError::Busy)
            ),
            "same id"
        );
        for h in handles {
            assert!(h.join().expect("join").expect("run").ok);
        }

        let limited = Runner::new(Limits {
            max_running: 4,
            per_minute: 2,
        });
        let quick = shell("q", "true", Duration::from_secs(5));
        assert!(limited.run(&quick, &p, Path::new("/")).is_ok());
        assert!(limited.run(&quick, &p, Path::new("/")).is_ok());
        assert!(matches!(
            limited.run(&quick, &p, Path::new("/")),
            Err(RunError::RateLimited)
        ));
    }
}
