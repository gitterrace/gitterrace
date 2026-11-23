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
        findings: None,
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

/// The `review` markdown is the shared vector file's, and the JS side reads the same file for
/// its `source:` lines.
#[test]
fn review_markdown_matches_the_shared_vectors() {
    use gitterrace_types::AnchorStatus;

    use crate::payload::{ReviewFinding, review_markdown};

    let text = include_str!("../../testdata/review-vectors.json");
    let rows: Vec<serde_json::Value> = serde_json::from_str(text).expect("vectors");
    assert_eq!(rows.len(), 2, "both vectors");
    for row in rows {
        let entries: Vec<ReviewFinding> = row["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|f| ReviewFinding {
                n: f["n"].as_u64().expect("n") as usize,
                kind: f["kind"].as_str().expect("kind").into(),
                path: f["path"].as_str().map(Into::into),
                lines: f["lines"].as_array().map(|l| Lines {
                    start: l[0].as_u64().expect("start") as u32,
                    end: l[1].as_u64().expect("end") as u32,
                }),
                sha: f["sha"].as_str().expect("sha").into(),
                permalink: f["permalink"].as_str().expect("link").into(),
                note: f["note"].as_str().expect("note").into(),
                status: f["status"].as_str().expect("status").parse::<AnchorStatus>().expect("ok"),
                text: f["text"].as_str().map(Into::into),
                text_truncated: f["text"].as_str().map(|_| false),
            })
            .collect();
        let md = review_markdown(
            row["repo"].as_str().expect("repo"),
            row["short"].as_str().expect("short"),
            &entries,
        );
        assert_eq!(md, row["markdown"].as_str().expect("markdown"), "{}", row["name"]);
    }
}

fn finding(
    path: Option<&str>,
    lines: Option<(u32, u32)>,
    sha: &str,
    note: &str,
) -> gitterrace_types::api::Finding {
    use gitterrace_types::AnchorBasis;
    use gitterrace_types::api::{Finding, FindingAnchor, FindingLines};
    Finding {
        added: 1,
        anchor: FindingAnchor {
            checked_against: AnchorBasis::Head,
            now_lines: None,
            now_path: path.map(Into::into),
            status: gitterrace_types::AnchorStatus::Untouched,
        },
        id: "f-00000000".into(),
        kind: if lines.is_some() { "line" } else { "file" }.into(),
        lines: lines.map(|(start, end)| FindingLines { end, start }),
        note: note.into(),
        path: path.map(Into::into),
        query: None,
        rev: "main".into(),
        sha: sha.into(),
    }
}

#[test]
fn a_review_payload_reads_its_text_from_git_and_sets_the_env() {
    let (t, sha) = hostile_repo();
    let repo = Repo::discover(t.path()).expect("open");
    let basket = [
        finding(Some("src/a.rs"), Some((2, 3)), &sha, "check $(touch pwned)\nnow"),
        finding(Some("src/a.rs"), None, &sha, ""),
    ];
    let p = Payload::build_review(&repo, "x", &basket, "o", LinkStyle::Gitterrace).expect("review");
    assert_eq!(p.kind, Kind::Review);
    assert_eq!((p.path.as_deref(), p.lines, p.text.as_deref()), (None, None, None));
    assert_eq!(p.title, "Review: 2 findings");
    assert_eq!(p.findings.len(), 2);
    let want = "line 2 $(touch pwned)\nline 3 $(touch pwned)\n";
    assert_eq!(p.findings[0].text.as_deref(), Some(want));
    assert_eq!(p.findings[1].text, None);
    assert_eq!(p.markdown.matches("source: ").count(), 2, "one source line per finding");
    assert!(p.markdown.contains("check $(touch pwned) now"), "a note is one line");
    let json = serde_json::to_value(&p).expect("json");
    assert_eq!(json["kind"], "review");
    assert_eq!(json["findings"][0]["n"], 1);
    assert_eq!(json["findings"][0]["status"], "untouched");
    let env = p.env();
    assert_eq!(env["GITTERRACE_FINDINGS"], "2");
    assert_eq!(env["GITTERRACE_PATH"], "src/a.rs");
    assert_eq!(env["GITTERRACE_LINE_START"], "2");
    assert_eq!(env["GITTERRACE_KIND"], "review");
    assert!(Payload::build_review(&repo, "x", &[], "o", LinkStyle::Go).is_err());
}

/// Class guard: v1 stays byte for byte. No single-kind payload carries a `findings` key, and a
/// locator naming findings is refused for every other kind, so the new field cannot leak into
/// an old kind.
#[test]
fn every_single_kind_payload_is_free_of_the_review_fields() {
    let (t, _) = hostile_repo();
    let repo = Repo::discover(t.path()).expect("open");
    for kind in [Kind::File, Kind::Commit, Kind::Page, Kind::SearchHit, Kind::Insight] {
        let p = Payload::build(
            &repo,
            "x",
            kind,
            &loc(None, None),
            &gitterrace_testkit::base_url(),
            LinkStyle::Go,
        )
        .or_else(|_| {
            Payload::build(
                &repo,
                "x",
                kind,
                &loc(Some("src/a.rs"), None),
                &gitterrace_testkit::base_url(),
                LinkStyle::Go,
            )
        })
        .expect("build");
        let json = serde_json::to_string(&p).expect("json");
        assert!(!json.contains("\"findings\""), "{kind:?}: {json}");
        assert!(!p.env().contains_key("GITTERRACE_FINDINGS"));
        let mut asked = loc(None, None);
        asked.findings = Some(vec!["f-1".into()]);
        assert!(Payload::build(&repo, "x", kind, &asked, "o", LinkStyle::Go).is_err());
    }
    let review = Payload::build(&repo, "x", Kind::Review, &loc(None, None), "o", LinkStyle::Go);
    assert!(matches!(review, Err(PayloadError::BadLocator(_))));
}

#[test]
fn a_locator_with_findings_reads_and_an_unknown_field_still_fails() {
    let ok = r#"{"repo":"r","rev":"main","findings":["f-1"]}"#;
    assert!(serde_json::from_str::<Locator>(ok).is_ok());
    assert!(serde_json::from_str::<Locator>(r#"{"repo":"r","rev":"main","note":"x"}"#).is_err());
}
