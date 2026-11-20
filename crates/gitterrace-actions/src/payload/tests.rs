use gitterrace_testkit::git_at;
/// Pinned author and committer date for every commit in this module.
const T0: &str = "2026-10-30T12:00:00+00:00";

use gitterrace_git::Repo;
use gitterrace_types::LinkStyle;

use crate::config::Kind;
use crate::payload::{Lines, Locator, Payload, PayloadError, RunRequest};

/// One commit whose subject and file content carry shell and HTML payloads.
pub(crate) fn hostile_repo() -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().expect("tempdir");
    let d = t.path();
    git_at(d, T0, &["init", "-q", "-b", "main"]);
    git_at(d, T0, &["config", "user.name", "Eve <script>"]);
    git_at(d, T0, &["config", "user.email", "e@example.com"]);
    std::fs::create_dir(d.join("src")).expect("mkdir");
    let body =
        (1..=500).map(|i| format!("line {i} $(touch pwned)")).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(d.join("src/a.rs"), body).expect("write");
    git_at(d, T0, &["add", "-A"]);
    git_at(d, T0, &["commit", "-q", "-m", "fix: $(touch pwned) `id` \"><img onerror=x>"]);
    let sha = git_at(d, T0, &["rev-parse", "HEAD"]);
    (t, sha)
}

fn loc(path: Option<&str>, lines: Option<(u32, u32)>) -> Locator {
    Locator {
        repo: "r".into(),
        rev: "main".into(),
        path: path.map(Into::into),
        lines: lines.map(|(start, end)| Lines { start, end }),
        sha: None,
        query: None,
    }
}

#[test]
fn line_payload_is_read_from_git() {
    let (t, sha) = hostile_repo();
    let repo = Repo::discover(t.path()).expect("open");
    let p = Payload::build(
        &repo,
        "x",
        Kind::Line,
        &loc(Some("src/a.rs"), Some((2, 3))),
        &gitterrace_testkit::base_url(),
        LinkStyle::Go,
    )
    .expect("build");
    assert_eq!(p.sha, sha);
    assert_eq!(p.text.as_deref(), Some("line 2 $(touch pwned)\nline 3 $(touch pwned)\n"));
    assert_eq!(p.text_truncated, Some(false));
    let id = repo.id().expect("id");
    assert_eq!(
        p.permalink,
        format!("{}/go/{id}/{sha}/src/a.rs#L2-L3", gitterrace_testkit::base_url())
    );
    let l = loc(Some("src/a.rs"), Some((2, 3)));
    let gt = Payload::build(&repo, "x", Kind::Line, &l, "o", LinkStyle::Gitterrace);
    let gt = gt.expect("build");
    assert_eq!(gt.permalink, format!("gitterrace://{id}/{sha}/src/a.rs#L2-L3"));
    assert!(gt.markdown.contains(&gt.permalink), "markdown carries the styled link");
    assert_eq!(p.title, format!("a.rs:2-3 @ {}", gitterrace_git::short_id(&sha)));
    assert!(
        p.markdown.starts_with(&format!("**a.rs:2-3** @ {}\n", gitterrace_git::short_id(&sha)))
    );
    assert!(p.markdown.contains("```rs\nline 2"));
    // The last line is the machine line, and it reads back to this selection.
    let last = p.markdown.lines().last().expect("a line");
    let src = crate::parse_source(last).expect("source line");
    assert_eq!(src.sha, sha);
    assert_eq!(src.path.as_deref(), Some("src/a.rs"));
    assert_eq!(src.lines, Some(Lines { start: 2, end: 3 }));
    assert_eq!(p.commit.as_ref().expect("commit").date, "2026-10-30T12:00:00Z");

    let json = serde_json::to_value(&p).expect("json");
    assert_eq!(json["v"], 1);
    assert_eq!(json["kind"], "line");
    assert!(json.get("query").is_none(), "absent, not null");
    let env = p.env();
    assert_eq!(env["GITTERRACE_LINE_START"], "2");
    assert!(!env.contains_key("GITTERRACE_TEXT"));
}

#[test]
fn caps_and_commit_kind() {
    let (t, _) = hostile_repo();
    let repo = Repo::discover(t.path()).expect("open");
    let p = Payload::build(
        &repo,
        "x",
        Kind::Line,
        &loc(Some("src/a.rs"), Some((1, 500))),
        "o",
        LinkStyle::Go,
    )
    .expect("build");
    assert_eq!(p.text.as_deref().expect("text").lines().count(), 400);
    assert_eq!(p.text_truncated, Some(true));

    let c = Payload::build(&repo, "x", Kind::Commit, &loc(None, None), "o", LinkStyle::Go);
    let c = c.expect("build");
    let json = serde_json::to_value(&c).expect("json");
    for absent in ["path", "lines", "text", "text_truncated"] {
        assert!(json.get(absent).is_none(), "{absent}");
    }
    assert!(c.title.contains("$(touch pwned)"), "title is data, kept verbatim");
    assert!(!c.env()["GITTERRACE_TITLE"].contains('\n'));
}

#[test]
fn bad_locators_are_refused() {
    let (t, _) = hostile_repo();
    let repo = Repo::discover(t.path()).expect("open");
    let bad = |kind, l: Locator| {
        matches!(
            Payload::build(&repo, "x", kind, &l, "o", LinkStyle::Go),
            Err(PayloadError::BadLocator(_))
        )
    };
    assert!(bad(Kind::File, loc(Some("../../etc/passwd"), None)));
    assert!(
        bad(Kind::File, loc(Some("/etc/passwd"), None))
            || bad(Kind::File, loc(Some("etc/passwd"), None))
    );
    assert!(bad(Kind::File, loc(Some("src/../src/a.rs"), None)));
    assert!(bad(Kind::Line, loc(Some("src/a.rs"), Some((0, 2)))));
    assert!(bad(Kind::Line, loc(Some("src/a.rs"), Some((5, 2)))));
    assert!(bad(Kind::Line, loc(Some("src/a.rs"), Some((900, 901)))));
    assert!(bad(Kind::Line, loc(Some("src/a.rs"), None)));
    assert!(bad(Kind::File, loc(None, None)));
    let mut r = loc(None, None);
    r.rev = "--upload-pack=x".into();
    assert!(bad(Kind::Commit, r));
    let mut r = loc(None, None);
    r.rev = "nope".into();
    assert!(bad(Kind::Commit, r));
}

#[test]
fn request_schema_is_strict() {
    let ok = concat!(
        r#"{"action":"a","kind":"line","locator":{"repo":"r","rev":"main","path":"x","#,
        r#""lines":{"start":1,"end":2}}}"#,
    );
    assert!(serde_json::from_str::<RunRequest>(ok).is_ok());
    for bad in [
        r#"{"action":"a","kind":"line","locator":{"repo":"r","rev":"main"},"command":["sh"]}"#,
        r#"{"action":"a","kind":"line","locator":{"repo":"r","rev":"main","cwd":"/"}}"#,
        r#"{"action":"a","kind":"shell","locator":{"repo":"r","rev":"main"}}"#,
        r#"{"action":"a","kind":"line"}"#,
    ] {
        assert!(serde_json::from_str::<RunRequest>(bad).is_err(), "{bad}");
    }
}
