//! With `NO_COLOR` set, or with output piped, the command line prints no escape sequences.
//!
//! Output here is always piped, so the `NO_COLOR` case also forces a color request through
//! `CLICOLOR_FORCE`, which a plain pipe alone would not exercise.
#![expect(clippy::expect_used, reason = "test setup")]

use std::process::Command;

fn run(args: &[&str], no_color: bool) -> (String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_git-terrace"));
    cmd.args(args)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE");
    if no_color {
        cmd.env("NO_COLOR", "1").env("CLICOLOR_FORCE", "1");
    }
    let out = cmd.output().expect("run git-terrace");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn assert_plain(what: &str, (out, err): &(String, String)) {
    for (name, text) in [("stdout", out), ("stderr", err)] {
        assert!(
            !text.contains('\x1b'),
            "{what} {name} has an escape sequence: {text:?}"
        );
    }
}

#[test]
fn help_is_plain_text_with_no_escape_sequences() {
    for no_color in [false, true] {
        let got = run(&["--help"], no_color);
        assert_plain("--help", &got);
        assert!(
            got.0.contains("Usage:"),
            "help has a usage section: {}",
            got.0
        );
        assert!(
            got.0.contains("Commands:"),
            "help lists commands: {}",
            got.0
        );
    }
}

#[test]
fn subcommand_help_and_version_are_plain() {
    for args in [
        &["open", "--help"][..],
        &["wrapped", "--help"],
        &["--version"],
    ] {
        for no_color in [false, true] {
            assert_plain(&args.join(" "), &run(args, no_color));
        }
    }
}

#[test]
fn a_usage_error_is_plain() {
    for no_color in [false, true] {
        let got = run(&["--no-such-flag"], no_color);
        assert_plain("usage error", &got);
        assert!(!got.1.is_empty(), "the error says what went wrong");
    }
}

#[test]
fn config_output_is_plain() {
    let dir = tempfile::tempdir().expect("temp dir");
    let cfg = dir.path().to_str().expect("utf-8 path");
    for no_color in [false, true] {
        assert_plain("config", &run(&["--config", cfg, "config"], no_color));
    }
}
