//! Every list of commits, and the branches, tags and remotes on `/refs`, renders the one commit
//! row `commit_line.rs` writes: avatar, subject, author and time, then the commit id. A row
//! written by hand drifts in order and spacing, which is how the refs page once showed the id
//! first and the log showed it last.

use std::path::Path;

use gitterrace_testkit::{Fixture, page};

/// Markup only `commit_line.rs` writes: the row and its main column.
const OWNED: &[&str] = &[
    "class=\"crow",
    "class=\"cmain",
    "class=\\\"crow",
    "class=\\\"cmain",
];

fn owned_in(code: &str) -> Vec<&'static str> {
    OWNED.iter().copied().filter(|m| code.contains(m)).collect()
}

/// A table row that holds a commit id link next to a subject: a commit row shaped as a table.
fn table_commits(body: &str) -> usize {
    body.split("<tr")
        .skip(1)
        .filter(|r| {
            let r = r.split("</tr>").next().unwrap_or_default();
            r.contains("class=\"sha\"") && r.contains("class=\"subj\"")
        })
        .count()
}

#[test]
fn a_hand_written_commit_row_is_caught() {
    assert_eq!(
        owned_in("<li class=\"crow\"><div class=\"cmain\">"),
        OWNED[..2]
    );
    assert_eq!(owned_in("\"<li class=\\\"crow\\\">\""), ["class=\\\"crow"]);
    let table = "<tr><td>v1</td><td><span class=\"subj\">x</span></td>\
                 <td><a class=\"sha\" href=\"/commit/1\">1</a></td></tr>";
    assert_eq!(table_commits(table), 1);
    assert!(owned_in("<li class=\"row\">").is_empty());
}

/// The markup lives in `commit_line.rs` and `commit_line.html` and nowhere else in the crate.
#[test]
fn only_the_shared_row_writes_a_commit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    let mut stack = vec![
        root.join("src"),
        root.join("templates"),
        root.join("static"),
    ];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            // The owner, the stylesheet that styles it and test files that read it back.
            if name.as_deref().is_some_and(|n| {
                n == "commit_line.rs"
                    || n == "commit_line.html"
                    || n == "app.css"
                    || n.ends_with("tests.rs")
            }) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            let found = owned_in(code);
            if !found.is_empty() {
                bad.push(format!("{}: {found:?}", p.display()));
            }
        }
    }
    assert!(bad.is_empty(), "hand-written commit row in {bad:?}");
}

/// The log, file history, refs, tag and compare pages list commits in the shared row only.
#[tokio::test]
async fn commit_lists_share_one_row() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path())
        .write("a.rs", "fn main() {}\n")
        .commit("feat: first")
        .tag("v1", "first release")
        .write("a.rs", "fn main() {\n}\n")
        .commit("fix: second");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    gitterrace_testkit::oracle::ready(&router).await;
    let mut fails = Vec::new();
    for (p, want) in [
        ("/log/HEAD", 2),
        ("/history/a.rs", 2),
        ("/refs", 2),
        ("/tag/v1", 1),
        ("/compare?base=v1&head=HEAD", 1),
    ] {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        let rows = body.matches("<li class=\"crow\">").count();
        if rows < want {
            fails.push(format!("{p}: {rows} shared rows, want {want}"));
        }
        if table_commits(&body) > 0 {
            fails.push(format!("{p}: a commit in a table row"));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
