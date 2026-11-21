use std::path::Path;
use std::time::{Duration, Instant};

use gitterrace_git::Repo;
use gitterrace_types::LinkStyle;

use crate::config::{Action, Cwd, Kind, Output, Stdin};
use crate::payload::{Lines, Locator, Payload};
use crate::runner::{Limits, RunError, Runner, spawn_argv};

fn shell(id: &str, script: &str, timeout: Duration) -> Action {
    Action {
        id: id.into(),
        label: id.into(),
        kinds: vec![Kind::Line, Kind::Commit],
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        shell: true,
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
    Payload::build(repo, "t", kind, &loc, &gitterrace_testkit::base_url(), LinkStyle::Go)
        .expect("payload")
}

fn fixture() -> (tempfile::TempDir, Repo) {
    let (t, _) = crate::payload::tests::hostile_repo();
    let r = Repo::discover(t.path()).expect("open");
    (t, r)
}

/// Shell text with braces runs: the script reaches `sh` untouched.
#[test]
fn shell_braces_run_verbatim() {
    let (_t, repo) = fixture();
    let p = payload(&repo, Kind::Line);
    let a = shell(
        "b",
        "cat >/dev/null; echo ${HOME} | awk '{print}'; f() { :; }; f",
        Duration::from_secs(5),
    );
    let r = Runner::new(Limits::default()).run(&a, &p, &repo.root()).expect("run");
    assert!(r.ok, "{r:?}");
    assert_eq!(r.first_line, std::env::var("HOME").expect("HOME"));
}

/// Whatever the loader accepts, the runner builds an argv for: load and run never
/// disagree on the template rules. The corpus is the shipped starter's enabled entries, every
/// placeholder and modifier, and shell text full of braces.
#[test]
fn every_loaded_action_builds_its_argv() {
    let (_t, repo) = fixture();
    let corpus = [
            crate::config::Config::STARTER.to_string(),
            r#"version = 1
[[action]]
id = "awk"
label = "Awk"
kinds = ["line"]
shell = true
command = 'echo ${HOME} | awk "{print}"; f() { x; }; echo {{ }} {'

[[action]]
id = "all"
label = "All"
kinds = ["line", "commit"]
command = ["/bin/echo", "x{{y}}", "--", "{title}", "{markdown}", "{text}", "{path}", "{line_start}", "{line_end}", "{sha}", "{url}", "{url:url}", "{repo}", "{root}", "{kind}", "{var.u}"]
[action.vars]
u = "https://x/{y}"

[[script]]
id = "s"
shell = true
command = 'printf "%s" "${GITTERRACE_TITLE}" | sed "s/{a}/b/"'

[[action]]
id = "uses"
label = "Uses"
kinds = ["page"]
script = "s"
"#
            .to_string(),
        ];
    let mut n = 0;
    for text in &corpus {
        let c = crate::config::Config::parse(text, Path::new("/")).expect("corpus loads");
        for kind in [Kind::Line, Kind::Commit] {
            let mut p = payload(&repo, kind);
            for a in &c.actions {
                p.vars.clone_from(&a.vars);
                let argv = spawn_argv(a, &p)
                    .unwrap_or_else(|e| panic!("{} loads but does not run: {e}", a.id));
                assert_eq!(argv.len(), a.argv.len(), "{}", a.id);
                n += 1;
            }
        }
    }
    assert!(n >= 6, "corpus too small: {n}");
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
        .run(&shell("a", &script, Duration::from_secs(5)), &p, &repo.root())
        .expect("run");
    assert!(r.ok, "{r:?}");
    assert_eq!(r.first_line, "first");
    let stdin = std::fs::read(t.path().join("seen.json")).expect("stdin");
    assert_eq!(stdin, serde_json::to_vec(&p).expect("json"));
    let env = std::fs::read_to_string(t.path().join("seen.env")).expect("env");
    let allowed = ["PATH", "HOME", "USER", "LANG", "TMPDIR", "PWD", "SHLVL", "OLDPWD", "_"];
    for line in env.lines() {
        let name = line.split('=').next().unwrap_or_default();
        assert!(
            allowed.contains(&name) || name.starts_with("LC_") || name.starts_with("GITTERRACE_"),
            "leaked {name}"
        );
    }
    assert!(env.contains("GITTERRACE_PATH=src/a.rs"));
    assert_eq!(
        std::fs::read_to_string(t.path().join("seen.pwd")).expect("pwd").trim(),
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
    a.vars = [("vault", "Notes"), ("path", "/evil"), ("home", "/evil"), ("var_path", "x")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let p = payload(&repo, Kind::Line);
    let r = Runner::new(Limits::default()).run(&a, &p, &repo.root()).expect("run");
    assert!(r.ok, "{r:?}");
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(t.path().join("vars.json")).expect("stdin"))
            .expect("json");
    assert_eq!(json["v"], 1);
    assert_eq!(json["vars"]["vault"], "Notes");
    assert_eq!(json["vars"]["path"], "/evil");
    assert_eq!(json["path"], "src/a.rs");
    let env = std::fs::read_to_string(t.path().join("vars.env")).expect("env");
    let get =
        |k: &str| env.lines().find_map(|l| l.strip_prefix(&format!("{k}=")).map(str::to_string));
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
            &shell("a", "cat >/dev/null; echo \"$GITTERRACE_TITLE\"", Duration::from_secs(5)),
            &p,
            &repo.root(),
        )
        .expect("run");
    assert!(r.ok);
    assert!(r.first_line.contains("$(touch pwned)"), "printed literally: {}", r.first_line);
    assert!(!t.path().join("pwned").exists());
}

#[test]
fn failure_reports_stderr_and_exit() {
    let (_t, repo) = fixture();
    let r = Runner::new(Limits::default())
        .run(
            &shell("a", "echo out; echo boom >&2; exit 3", Duration::from_secs(5)),
            &payload(&repo, Kind::Commit),
            &repo.root(),
        )
        .expect("run");
    assert_eq!((r.ok, r.exit, r.first_line.as_str()), (false, Some(3), "boom"));
}

#[test]
fn timeout_kills_the_whole_group() {
    let (t, repo) = fixture();
    let pidfile = t.path().join("grandchild.pid");
    // The pid is published by rename, so a reader never sees it half written, and the timeout is
    // wide enough that the grandchild has started on a loaded machine before it fires.
    let script = format!(
        "sh -c 'echo $$ > \"{0}.tmp\"; mv \"{0}.tmp\" \"{0}\"; sleep 30' & sleep 30",
        pidfile.display()
    );
    let start = Instant::now();
    let r = Runner::new(Limits::default())
        .run(
            &shell("a", &script, Duration::from_millis(2000)),
            &payload(&repo, Kind::Commit),
            &repo.root(),
        )
        .expect("run");
    assert!(r.timed_out && !r.ok);
    assert!(start.elapsed() < Duration::from_secs(10), "{:?}", start.elapsed());
    let pid: i32 = std::fs::read_to_string(&pidfile).expect("pid").trim().parse().expect("int");
    // The kernel reaps the signalled group soon after, on a loaded runner later.
    gitterrace_testkit::wait::process_gone_blocking("the grandchild is reaped", &pid.to_string());
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

/// `limits.action_output_kb` reaches the runner: a smaller setting keeps less.
#[test]
fn output_limit_is_the_configured_one() {
    let (_t, repo) = fixture();
    let r = Runner::new(Limits { output_bytes: 1000, ..Limits::default() })
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
    assert_eq!(r.stdout.len(), 1000);
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
    // All four hold a slot before the fifth asks.
    gitterrace_testkit::wait::until_blocking("four runs hold a slot", || {
        runner.state.lock().is_ok_and(|s| s.running.len() == 4)
    });
    let p = payload(&repo, Kind::Commit);
    assert!(
        matches!(
            runner.run(&shell("a9", "true", Duration::from_secs(5)), &p, Path::new("/")),
            Err(RunError::Busy)
        ),
        "5th concurrent"
    );
    assert!(
        matches!(
            runner.run(&shell("a0", "true", Duration::from_secs(5)), &p, Path::new("/")),
            Err(RunError::Busy)
        ),
        "same id"
    );
    for h in handles {
        assert!(h.join().expect("join").expect("run").ok);
    }

    let limited = Runner::new(Limits { per_minute: 2, ..Limits::default() });
    let quick = shell("q", "true", Duration::from_secs(5));
    assert!(limited.run(&quick, &p, Path::new("/")).is_ok());
    assert!(limited.run(&quick, &p, Path::new("/")).is_ok());
    assert!(matches!(limited.run(&quick, &p, Path::new("/")), Err(RunError::RateLimited)));
}

/// Runs `argv` (placeholders included) with a recorder in front: `/bin/sh -c` writes each
/// argument it gets on its own NUL-terminated record, and stdin beside it.
fn record(t: &Path, args: &[&str], stdin: Stdin, p: &Payload) -> Result<Vec<String>, RunError> {
    let out = t.join("argv");
    let script = format!("printf '%s\\0' \"$@\" > '{0}'; cat > '{0}.in'", out.display());
    let mut a = shell("rec", &script, Duration::from_secs(5));
    // A recorder: `sh -c` only prints the template arguments it is handed, so this is an
    // argv template action, and its arguments are expanded.
    a.shell = false;
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
    for title in ["--help", "$(touch PWNED)", "a b; touch PWNED", "`touch PWNED`"] {
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
    assert!(record(t.path(), &["--", "{title}"], Stdin::None, &p).is_ok(), "at cap");
    p.title.push('x');
    let e = record(t.path(), &["--", "{title}"], Stdin::None, &p).expect_err("over");
    assert!(
        matches!(&e, RunError::Refused(m) if m.contains("{title}") && m.contains("64 KiB")),
        "{e}"
    );
    p.title = "x".repeat(60 * 1024);
    let five = ["--", "{title}", "{title}", "{title}", "{title}", "{title}"];
    let e = record(t.path(), &five, Stdin::None, &p).expect_err("total");
    assert!(matches!(&e, RunError::Refused(m) if m.contains("256 KiB")), "{e}");
    assert!(record(t.path(), &five[..5], Stdin::None, &p).is_ok(), "under total");
    p.title = "a\0b".into();
    let e = record(t.path(), &["--", "{title}"], Stdin::None, &p).expect_err("nul");
    assert!(matches!(&e, RunError::Refused(m) if m.contains("NUL")), "{e}");
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
    let got =
        record(t.path(), &["x://new?title={title:url}&notes={markdown:url}"], Stdin::None, &p)
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
    assert_eq!(read(), [] as [u8; 0]);
}

/// A timed-out run gives its slot back when `run` returns, not when the kernel reaps its group:
/// with one slot, the next run starts right away even while the killed grandchild may linger.
#[test]
fn a_timed_out_run_frees_its_slot_on_return() {
    let (_t, repo) = fixture();
    let runner = Runner::new(Limits { max_running: 1, ..Limits::default() });
    let p = payload(&repo, Kind::Commit);
    let slow = shell("slow", "sh -c 'sleep 30' & sleep 30", Duration::from_millis(200));
    let r = runner.run(&slow, &p, &repo.root()).expect("run");
    assert!(r.timed_out);
    let next =
        runner.run(&shell("next", "cat >/dev/null", Duration::from_secs(5)), &p, &repo.root());
    assert!(next.is_ok_and(|o| o.ok), "the slot was still held after the time-out");
}

/// A script that never reads stdin must not stall the run, even when the payload is larger than
/// a pipe buffer: the write sits on its own thread.
#[test]
fn a_script_that_ignores_a_large_stdin_still_finishes() {
    let (_t, repo) = fixture();
    let mut p = payload(&repo, Kind::Commit);
    p.markdown = "x".repeat(1 << 20);
    let mut a = shell("a", "sleep 0.1; echo done", Duration::from_secs(10));
    a.stdin = Stdin::Markdown;
    let began = Instant::now();
    let r = Runner::new(Limits::default()).run(&a, &p, &repo.root()).expect("run");
    assert!(r.ok && r.first_line == "done", "{r:?}");
    assert!(began.elapsed() < Duration::from_secs(5));
}
