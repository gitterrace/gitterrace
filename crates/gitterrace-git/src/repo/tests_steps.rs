//! A committed-only rev walks a bounded number of ancestor steps, however the URL spells them.

use std::fmt::Write as _;

use gitterrace_testkit::git_at;

use crate::Repo;
use crate::repo::tests::T0;

/// A linear `main` of `n` commits built with one fast-import.
fn linear(d: &std::path::Path, n: usize) {
    git_at(d, T0, &["init", "-q", "-b", "main"]);
    let mut script = String::new();
    for i in 1..=n {
        let t = 1_700_000_000 + i;
        let from = if i > 1 { format!("from :{}\n", i - 1) } else { String::new() };
        write!(
            script,
            "commit refs/heads/main\nmark :{i}\ncommitter t <t@t> {t} +0000\ndata 1\nx\n{from}"
        )
        .expect("write");
    }
    gitterrace_testkit::git_stdin(d, &["fast-import", "--quiet"], script.as_bytes());
}

/// Guard class: any rev whose steps add up past the cap, in one step or spread over many, is
/// refused before the walk starts, so it costs no more than one at the cap. Red when a step
/// count reaches the parent walk uncapped (each step reads one commit from disk).
#[test]
fn ancestor_steps_above_the_cap_are_refused_fast() {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    linear(d, 3_000);
    let open = Repo::discover(d).expect("repo");
    let repo = Repo::discover(d).expect("repo").committed_only();
    assert_eq!(
        repo.resolve("main~1000").expect("at cap"),
        open.resolve("main~1000").expect("open")
    );
    assert_eq!(repo.resolve("main~0").expect("zero"), open.resolve("main").expect("tip"));
    assert_eq!(
        repo.resolve("main~500~500").expect("chained at cap"),
        open.resolve("main~1000").expect("open")
    );
    let deep = format!("main{}", "~1".repeat(1_001));
    assert!(open.resolve("main~2999").is_ok(), "the history is deep enough");
    for rev in ["main~1001", "main~2999", "main~600~600", &deep, "HEAD^1001"] {
        assert!(
            matches!(repo.resolve(rev), Err(crate::Error::RevNotFound(_))),
            "{rev:.20} resolved"
        );
    }
    // Uncapped, 200 walks of 2999 steps read 600k commits: seconds, not milliseconds.
    let started = std::time::Instant::now();
    for _ in 0..200 {
        assert!(repo.resolve("main~2999").is_err());
    }
    let spent = started.elapsed();
    assert!(spent < std::time::Duration::from_millis(500), "refusals took {spent:?}");
}
