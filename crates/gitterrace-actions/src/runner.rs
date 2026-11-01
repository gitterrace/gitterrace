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

use crate::config::{Action, Cwd, Piece, REPO_FIELDS, Stdin, field_parts, pieces, url_encode};
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
/// One expanded placeholder value. Larger values belong on stdin, not in argv.
const MAX_VALUE_BYTES: usize = 64 * 1024;
/// The whole expanded argv, well under every OS's `ARG_MAX` with the environment beside it.
const MAX_ARGV_BYTES: usize = 256 * 1024;

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
        let argv = expand(&action.argv, &payload)?;
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
                Cwd::Home => std::env::var_os("HOME").map_or_else(|| "/".into(), Into::into),
                Cwd::Path(p) => p.clone(),
            })
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
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
        // Vars first, payload last: even a colliding name could not replace a payload value.
        cmd.envs(payload.var_env());
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
        let ps = pieces(el).map_err(RunError::Refused)?;
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
                        Some("url") => url_encode(&raw),
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

fn supervise(mut child: Child, stdin: Option<Vec<u8>>, timeout: Duration) -> Outcome {
    let pgid = Pid::from_child(&child);
    // Writing on its own thread: a script that never reads stdin must not stall us.
    if let (Some(mut pipe), Some(stdin)) = (child.stdin.take(), stdin) {
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

    use crate::config::{Action, Cwd, Kind, Output, Stdin};
    use crate::payload::{Lines, Locator, Payload};
    use crate::runner::{Limits, RunError, Runner};

    fn shell(id: &str, script: &str, timeout: Duration) -> Action {
        Action {
            id: id.into(),
            label: id.into(),
            kinds: vec![Kind::Line, Kind::Commit],
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            stdin: Stdin::Json,
            cwd: Cwd::Repo,
            timeout,
            output: Output::Toast,
            key: None,
            env: vec![],
            vars: std::collections::BTreeMap::new(),
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
    fn vars_reach_env_and_stdin_but_never_shadow() {
        let (t, repo) = fixture();
        let out = t.path().join("vars");
        let mut a = shell(
            "v",
            &format!("cat > '{0}.json'; env > '{0}.env'", out.display()),
            Duration::from_secs(5),
        );
        // Names chosen to collide with the payload vars, PATH and HOME if unprefixed.
        a.vars = [
            ("vault", "Notes"),
            ("path", "/evil"),
            ("home", "/evil"),
            ("var_path", "x"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let p = payload(&repo, Kind::Line);
        let r = Runner::new(Limits::default())
            .run(&a, &p, &repo.root())
            .expect("run");
        assert!(r.ok, "{r:?}");
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(t.path().join("vars.json")).expect("stdin"))
                .expect("json");
        assert_eq!(json["v"], 1);
        assert_eq!(json["vars"]["vault"], "Notes");
        assert_eq!(json["vars"]["path"], "/evil");
        assert_eq!(json["path"], "src/a.rs");
        let env = std::fs::read_to_string(t.path().join("vars.env")).expect("env");
        let get = |k: &str| {
            env.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")).map(str::to_string))
        };
        assert_eq!(get("GITTERRACE_VAR_VAULT").as_deref(), Some("Notes"));
        assert_eq!(get("GITTERRACE_VAR_PATH").as_deref(), Some("/evil"));
        assert_eq!(get("GITTERRACE_VAR_VAR_PATH").as_deref(), Some("x"));
        assert_eq!(get("GITTERRACE_PATH").as_deref(), Some("src/a.rs"));
        assert_ne!(get("PATH").as_deref(), Some("/evil"));
        assert_ne!(get("HOME").as_deref(), Some("/evil"));
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

    /// Runs `argv` (placeholders included) with a recorder in front: `/bin/sh -c` writes each
    /// argument it gets on its own NUL-terminated record, and stdin beside it.
    fn record(t: &Path, args: &[&str], stdin: Stdin, p: &Payload) -> Result<Vec<String>, RunError> {
        let out = t.join("argv");
        let script = format!(
            "printf '%s\\0' \"$@\" > '{0}'; cat > '{0}.in'",
            out.display()
        );
        let mut a = shell("rec", &script, Duration::from_secs(5));
        a.argv.push("rec".into());
        a.argv.extend(args.iter().map(|s| (*s).to_string()));
        a.stdin = stdin;
        a.vars.clone_from(&p.vars);
        let r = Runner::new(Limits::default()).run(&a, p, t)?;
        assert!(r.ok, "{r:?}");
        let raw = std::fs::read_to_string(&out).expect("argv");
        Ok(raw.split_terminator('\0').map(str::to_string).collect())
    }

    #[test]
    fn child_argv_is_the_template_filled_in() {
        let (t, repo) = fixture();
        let mut p = payload(&repo, Kind::Line);
        p.vars.insert("tag".into(), "x y".into());
        let got = record(
            t.path(),
            &[
                "add",
                "--notes",
                "{markdown}",
                "--line={line_start}",
                "{{lit}}",
                "#{var.tag}",
                "--",
                "{title}",
                "{path}",
            ],
            Stdin::Json,
            &p,
        )
        .expect("run");
        let want = [
            "add".to_string(),
            "--notes".into(),
            p.markdown.clone(),
            "--line=1".into(),
            "{lit}".into(),
            "#x y".into(),
            "--".into(),
            p.title.clone(),
            "src/a.rs".into(),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn hostile_titles_are_one_literal_argument() {
        let (t, repo) = fixture();
        let mut p = payload(&repo, Kind::Commit);
        for title in [
            "--help",
            "$(touch PWNED)",
            "a b; touch PWNED",
            "`touch PWNED`",
        ] {
            p.title = title.into();
            let got = record(t.path(), &["--", "{title}"], Stdin::None, &p).expect("run");
            assert_eq!(got, ["--", title]);
            assert!(!t.path().join("PWNED").exists(), "{title} ran");
        }
        // Without `--`, a value after a literal option that starts with `-` is refused.
        p.markdown = "-rf".into();
        let e = record(t.path(), &["--notes", "{markdown}"], Stdin::None, &p).expect_err("refused");
        assert!(e.to_string().contains("option"), "{e}");
    }

    #[test]
    fn caps_and_nul_refuse_the_run() {
        let (t, repo) = fixture();
        let mut p = payload(&repo, Kind::Commit);
        p.title = "x".repeat(64 * 1024);
        assert!(
            record(t.path(), &["--", "{title}"], Stdin::None, &p).is_ok(),
            "at cap"
        );
        p.title.push('x');
        let e = record(t.path(), &["--", "{title}"], Stdin::None, &p).expect_err("over");
        assert!(
            matches!(&e, RunError::Refused(m) if m.contains("{title}") && m.contains("64 KiB")),
            "{e}"
        );
        p.title = "x".repeat(60 * 1024);
        let five = ["--", "{title}", "{title}", "{title}", "{title}", "{title}"];
        let e = record(t.path(), &five, Stdin::None, &p).expect_err("total");
        assert!(
            matches!(&e, RunError::Refused(m) if m.contains("256 KiB")),
            "{e}"
        );
        assert!(
            record(t.path(), &five[..5], Stdin::None, &p).is_ok(),
            "under total"
        );
        p.title = "a\0b".into();
        let e = record(t.path(), &["--", "{title}"], Stdin::None, &p).expect_err("nul");
        assert!(
            matches!(&e, RunError::Refused(m) if m.contains("NUL")),
            "{e}"
        );
    }

    #[test]
    fn url_modifier_encodes_a_query_value() {
        let (t, repo) = fixture();
        let mut p = payload(&repo, Kind::Commit);
        for (title, want) in [
            ("a&b", "a%26b"),
            ("a=b", "a%3Db"),
            ("a#b", "a%23b"),
            ("a?b", "a%3Fb"),
            ("a/b", "a%2Fb"),
            ("a+b", "a%2Bb"),
            ("a b", "a%20b"),
            ("-._~Az09", "-._~Az09"),
            ("\u{e9}\u{1f600}", "%C3%A9%F0%9F%98%80"),
            ("a\nb", "a%0Ab"),
        ] {
            p.title = title.into();
            let got = record(t.path(), &["q={title:url}"], Stdin::None, &p).expect("run");
            assert_eq!(got, [format!("q={want}")], "{title:?}");
        }
        // A title cannot add a parameter of its own.
        p.title = "t&notes=x".into();
        p.markdown = "m".into();
        let got = record(
            t.path(),
            &["x://new?title={title:url}&notes={markdown:url}"],
            Stdin::None,
            &p,
        )
        .expect("run");
        assert_eq!(got, ["x://new?title=t%26notes%3Dx&notes=m"]);
        let query = got[0].split_once('?').expect("query").1;
        assert_eq!(query.matches("notes=").count(), 1, "{query}");
        // The cap counts the encoded value: 30 KiB of spaces is 90 KiB encoded.
        p.title = " ".repeat(30 * 1024);
        let e = record(t.path(), &["q={title:url}"], Stdin::None, &p).expect_err("cap");
        assert!(e.to_string().contains("64 KiB"), "{e}");
    }

    #[test]
    fn stdin_modes() {
        let (t, repo) = fixture();
        let p = payload(&repo, Kind::Line);
        let read = || std::fs::read(t.path().join("argv.in")).expect("stdin");
        record(t.path(), &[], Stdin::Json, &p).expect("json");
        let mut full = p.clone();
        full.vars.clear();
        assert_eq!(read(), serde_json::to_vec(&full).expect("json"));
        record(t.path(), &[], Stdin::Markdown, &p).expect("md");
        assert_eq!(read(), p.markdown.as_bytes());
        record(t.path(), &[], Stdin::None, &p).expect("none");
        assert!(read().is_empty());
    }
}
