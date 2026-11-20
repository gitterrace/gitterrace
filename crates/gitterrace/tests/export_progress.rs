//! `export` reports progress on stderr: plain lines when piped, nothing with `--quiet`, and
//! stdout keeps only the final count.
#![expect(clippy::expect_used, reason = "test setup")]

use std::process::Command;

use gitterrace_testkit::Fixture;

fn export(quiet: bool) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let fx = Fixture::init(&dir.path().join("r"))
        .write("a.txt", "a\n")
        .commit("one");
    let out = dir.path().join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_git-terrace"));
    cmd.arg("export")
        .arg(&out)
        .arg(fx.path())
        .arg("--config")
        .arg(dir.path().join("cfg"))
        .env("HOME", dir.path())
        // A saturated machine pauses the index build by design: the test is about the lines.
        .env("TEST_INDEX_LOAD", "0")
        .env("XDG_CACHE_HOME", dir.path().join("cache"));
    if quiet {
        cmd.arg("--quiet");
    }
    let o = cmd.output().expect("run git-terrace");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn a_piped_export_prints_phases_and_the_count_as_plain_lines() {
    let (out, err) = export(false);
    assert!(out.starts_with("wrote "), "{out}");
    assert!(
        err.contains("note: _headers carries the CSP"),
        "a host needs it: {err}"
    );
    assert!(!err.contains('\x1b') && !err.contains('\r'), "{err:?}");
    assert!(err.contains("building the cache for /"), "{err}");
    assert!(err.contains("writing pages ("), "{err}");
    // Any m:ss time: a loaded machine can take a minute, which is not what this checks.
    assert!(
        err.lines().any(|l| l.starts_with("wrote ")
            && l.split_once(" pages (")
                .and_then(|(_, t)| t.strip_suffix(')'))
                .and_then(|t| t.split_once(':'))
                .is_some_and(|(m, s)| m.parse::<u64>().is_ok() && s.len() == 2)),
        "{err}"
    );
}

#[test]
fn quiet_export_prints_no_progress() {
    let (out, err) = export(true);
    assert!(out.starts_with("wrote "), "{out}");
    assert!(!err.contains("writing pages"), "{err}");
    assert!(!err.contains("building the cache"), "{err}");
}
