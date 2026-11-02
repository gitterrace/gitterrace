//! A revision too far from the indexed branch gets a page that says so, never a load of every
//! commit it lacks: the API answers `needs_reindex`, HEAD keeps serving from the index, and a
//! revision within the cap is served as usual.

use std::fmt::Write as _;
use std::path::Path;

use gitterrace_testkit::{git, git_stdin, oracle, page};

/// `main` with a few commits, `near` 3 commits off it and `far` past the cap, built with one
/// fast-import (thousands of `git commit` calls would take minutes).
fn repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    let far = gitterrace_index::feed::FRESH_CAP + 5;
    let mut s = String::new();
    let mut mark = 0;
    let mut commit = |s: &mut String, branch: &str, from: Option<usize>, file: &str, n: usize| {
        mark += 1;
        let t = 1_700_000_000 + mark * 60;
        let body = format!("line {n}\n");
        let _ = write!(
            s,
            "commit refs/heads/{branch}\nmark :{mark}\n\
             author A <a@x.io> {t} +0000\ncommitter A <a@x.io> {t} +0000\n\
             data 8\nfeat: {n:02}\n"
        );
        if let Some(f) = from {
            let _ = writeln!(s, "from :{f}");
        }
        let _ = write!(s, "M 100644 inline {file}\ndata {}\n{body}\n", body.len());
        mark
    };
    let mut tip = None;
    for n in 0..5 {
        tip = Some(commit(&mut s, "main", tip, "a.txt", n));
    }
    let fork = tip;
    let mut near = fork;
    for n in 0..3 {
        near = Some(commit(&mut s, "near", near, "near.txt", n));
    }
    let mut far_tip = fork;
    for n in 0..far {
        far_tip = Some(commit(&mut s, "far", far_tip, "far.txt", n % 100));
    }
    git_stdin(dir, &["fast-import", "--quiet"], s.as_bytes());
    git(dir, &["checkout", "-q", "main"]);
}

#[test]
fn a_far_revision_asks_for_a_reindex_and_holds_nothing() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        repo(dir.path());
        let router = gitterrace_web::app_router(dir.path()).expect("router");
        oracle::ready(&router).await;

        let (status, body) = page(&router, "/stats?rev=far").await;
        assert_eq!(status, 200);
        assert!(body.contains("need a reindex"), "far page missing: {body}");
        assert!(body.contains("indexed branch"), "{body}");
        assert!(
            !body.contains("data-progress"),
            "indexing page instead: {body}"
        );

        let (status, api) = page(&router, "/api/v1/insights/summary?rev=far").await;
        assert_eq!(status, 409, "{api}");
        assert!(api.contains("\"needs_reindex\""), "{api}");

        // Other views of that commit keep working.
        let (status, _) = page(&router, "/log/far").await;
        assert_eq!(status, 200);

        // HEAD still reads the index, and a revision within the cap is served.
        let (status, head) = page(&router, "/stats").await;
        assert_eq!(status, 200);
        assert!(!head.contains("need a reindex") && !head.contains("data-progress"));
        oracle::ready_at(&router, "near").await;
        let (status, near) = page(&router, "/api/v1/insights/summary?rev=near").await;
        assert_eq!(status, 200, "{near}");

        // The cache still holds HEAD's history only: nothing was indexed for `far`.
        let (status, again) = page(&router, "/stats?rev=far").await;
        assert_eq!(status, 200);
        assert!(again.contains("need a reindex"), "{again}");
    });
}
