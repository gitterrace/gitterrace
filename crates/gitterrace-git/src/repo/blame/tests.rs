//! Tests of blame and its history horizon.

use std::sync::atomic::AtomicBool;

/// Blame through the owned index is gix's own blame, entry for entry, on a history with
/// moves, edits under `.gitattributes` rules and a merge. Red when an option differs from
/// the shortcut's default (rewrites, since, ranges, algorithm) or attributes are dropped.
#[test]
fn the_owned_index_blame_is_gix_blame() -> crate::Result<()> {
    let dir = tempfile::tempdir().map_err(crate::error::git)?;
    let p = dir.path();
    let at = |args: &[&str]| gitterrace_testkit::git_at(p, "2024-01-02T12:00:00Z", args);
    let body = |tag: &str| (0..30).fold(String::new(), |b, l| b + &format!("{tag} {l}\n"));
    at(&["init", "-q", "-b", "main"]);
    std::fs::write(p.join(".gitattributes"), "*.txt text eol=crlf\n*.md diff=markdown\n")
        .map_err(crate::error::git)?;
    std::fs::write(p.join("a.txt"), body("a")).map_err(crate::error::git)?;
    std::fs::write(p.join("b.md"), body("b")).map_err(crate::error::git)?;
    at(&["add", "-A"]);
    at(&["commit", "-qm", "one"]);
    at(&["mv", "a.txt", "moved.txt"]);
    std::fs::write(p.join("moved.txt"), body("a") + "tail\n").map_err(crate::error::git)?;
    at(&["commit", "-qam", "move and edit"]);
    at(&["checkout", "-q", "-b", "side", "HEAD~1"]);
    std::fs::write(p.join("b.md"), "side\n".to_owned() + &body("b")).map_err(crate::error::git)?;
    at(&["commit", "-qam", "side edit"]);
    at(&["checkout", "-q", "main"]);
    at(&["merge", "-q", "--no-edit", "side"]);
    let repo = crate::Repo::discover(p)?;
    let gix = repo.repo();
    let tip = repo.resolve_id(&gix, "HEAD")?;
    for path in ["moved.txt", "b.md", ".gitattributes"] {
        let ours = super::blame_outcome(&gix, tip, path, u64::MAX)?;
        let theirs = gix
            .blame_file(path.into(), tip, gix::repository::blame_file::Options::default())
            .map_err(crate::error::git)?;
        assert_eq!(format!("{:?}", ours.entries), format!("{:?}", theirs.entries), "{path}");
    }
    Ok(())
}

/// The horizon walk reads through a store with no object cache and a delta cache of its
/// own even when the blame's repository has both, and finds the boundary for a nested
/// path through commits that share a root tree. Red when the walk goes back to the
/// repository's cached store, or the raw tree lookup misses a nested path.
#[test]
fn the_horizon_walk_owns_its_caches_and_finds_nested_paths() -> crate::Result<()> {
    let dir = tempfile::tempdir().map_err(crate::error::git)?;
    let p = dir.path();
    std::fs::create_dir_all(p.join("d/e")).map_err(crate::error::git)?;
    gitterrace_testkit::git_at(p, "2024-01-01T00:00:00Z", &["init", "-q", "-b", "main"]);
    for n in 0..4 {
        std::fs::write(p.join("d/e/f.txt"), "x".repeat(100) + &n.to_string())
            .map_err(crate::error::git)?;
        let date = format!("2024-01-01T00:0{n}:00Z");
        gitterrace_testkit::git_at(p, &date, &["add", "-A"]);
        gitterrace_testkit::git_at(p, &date, &["commit", "-qm", &format!("v{n}")]);
        gitterrace_testkit::git_at(p, &date, &["commit", "-q", "--allow-empty", "-m", "e"]);
    }
    let repo = crate::Repo::discover(p)?;
    let mut gix = repo.repo();
    gix.object_cache_size(1024 * 1024);
    let odb = super::horizon_odb(&gix);
    assert!(gix.objects.has_object_cache());
    assert!(!odb.has_object_cache() && odb.has_pack_cache());
    let tip = repo.resolve_id(&gix, "HEAD")?;
    assert_eq!(
        super::history_horizon(&gix, tip, "d/e/f.txt", 1000, &AtomicBool::new(false))?.since,
        None
    );
    // Two versions fit 250 bytes, the third (v1, at 00:01) does not.
    let since = super::history_horizon(&gix, tip, "d/e/f.txt", 250, &AtomicBool::new(false))?.since;
    assert_eq!(since, Some(1_704_067_260 + 1));
    assert_eq!(
        super::history_horizon(&gix, tip, "d/missing", 0, &AtomicBool::new(false))?.since,
        None
    );
    Ok(())
}

/// A file added late to a long history full of merges whose other side never touches it:
/// the horizon walk visits only the file's own line of commits, as blame does, never the
/// rest of the graph. Red when the walk goes back to every ancestor (a cold blame of a
/// header added once to linux took 90 s), follows a merge side that has the same version,
/// or keeps walking past the commit that added the file.
#[test]
fn the_horizon_walk_visits_only_the_files_own_history() -> crate::Result<()> {
    let dir = tempfile::tempdir().map_err(crate::error::git)?;
    let p = dir.path();
    let at = |n: usize, args: &[&str]| {
        let date = format!("2024-01-01T{:02}:{:02}:00Z", n / 60, n % 60);
        gitterrace_testkit::git_at(p, &date, args);
    };
    at(0, &["init", "-q", "-b", "main"]);
    // 100 commits of other work before the file exists.
    for n in 0..100 {
        std::fs::write(p.join("other.txt"), n.to_string()).map_err(crate::error::git)?;
        at(n, &["add", "-A"]);
        at(n, &["commit", "-qm", &format!("o{n}")]);
    }
    // The file, then 20 merges of side branches that change only other files.
    std::fs::write(p.join("f.txt"), "one\n").map_err(crate::error::git)?;
    at(100, &["add", "-A"]);
    at(100, &["commit", "-qm", "add f"]);
    for n in 0..20 {
        let t = 101 + n * 4;
        at(t, &["checkout", "-q", "-b", &format!("s{n}")]);
        for k in 0..3 {
            std::fs::write(p.join("side.txt"), format!("{n} {k}")).map_err(crate::error::git)?;
            at(t + k, &["add", "-A"]);
            at(t + k, &["commit", "-qm", "side"]);
        }
        at(t + 3, &["checkout", "-q", "main"]);
        at(t + 3, &["merge", "-q", "--no-ff", "--no-edit", &format!("s{n}")]);
    }
    let repo = crate::Repo::discover(p)?;
    let gix = repo.repo();
    let tip = repo.resolve_id(&gix, "HEAD")?;
    let horizon = super::history_horizon(&gix, tip, "f.txt", u64::MAX, &AtomicBool::new(false))?;
    // The 20 merges and the commit that added f.txt: the first parent always holds the
    // same version, so no side commit and nothing before the file is read.
    assert_eq!(horizon.commits, 21, "walked {} commits", horizon.commits);
    Ok(())
}

/// A file whose versions fit the budget is blamed in one pass and the horizon walk never
/// runs. Past the budget the walk runs and still bounds the blame. Red when every blame
/// walks the history first, which made a cold blame of a long-lived file read its
/// history twice, or when the counting pass stops bounding the blame.
#[test]
fn a_blame_that_fits_its_budget_skips_the_horizon_walk() -> crate::Result<()> {
    let dir = tempfile::tempdir().map_err(crate::error::git)?;
    let p = dir.path();
    gitterrace_testkit::git_at(p, "2024-01-01T00:00:00Z", &["init", "-q", "-b", "main"]);
    let mut body = String::new();
    for n in 0..10 {
        body.push_str(&"x".repeat(40));
        body.push_str(&n.to_string());
        body.push('\n');
        std::fs::write(p.join("f.txt"), &body).map_err(crate::error::git)?;
        let date = format!("2024-01-01T00:{n:02}:00Z");
        gitterrace_testkit::git_at(p, &date, &["add", "-A"]);
        gitterrace_testkit::git_at(p, &date, &["commit", "-qm", &format!("v{n}")]);
    }
    let repo = crate::Repo::discover(p)?;
    let gix = repo.repo();
    let tip = repo.resolve_id(&gix, "HEAD")?;
    let (fits, walked) = super::blame_bounded(&gix, tip, "f.txt", 10_000)?;
    assert_eq!(walked, 0, "walked {walked} commits");
    assert_eq!(fits.statistics.blobs_diffed, 9);
    let (bounded, walked) = super::blame_bounded(&gix, tip, "f.txt", 500)?;
    assert!(walked > 0);
    assert!(bounded.statistics.blobs_diffed < 9);
    Ok(())
}

/// A file with many large versions: blame diffs only the versions that fit its budget, so
/// the blobs gix holds stay bounded however long the history is, and every line is still
/// blamed. Red when the bound is dropped, not passed to gix, or counted past the boundary.
#[test]
fn blame_holds_only_the_versions_that_fit_its_budget() -> crate::Result<()> {
    let dir = tempfile::tempdir().map_err(crate::error::git)?;
    let p = dir.path();
    let version =
        |n: usize| (0..4000).fold(format!("v{n}\n"), |b, l| b + &format!("line {l} xxxxxxxxxx\n"));
    gitterrace_testkit::git_at(p, "2024-01-01T00:00:00Z", &["init", "-q", "-b", "main"]);
    for n in 0..40 {
        std::fs::write(p.join("big.txt"), version(n)).map_err(crate::error::git)?;
        let date = format!("2024-01-01T{:02}:{:02}:00Z", n / 60, n % 60);
        gitterrace_testkit::git_at(p, &date, &["add", "big.txt"]);
        gitterrace_testkit::git_at(p, &date, &["commit", "-qm", &format!("v{n}")]);
    }
    let repo = crate::Repo::discover(p)?;
    let gix = repo.repo();
    let tip = repo.resolve_id(&gix, "HEAD")?;
    let size = version(39).len() as u64;
    let full = super::blame_outcome(&gix, tip, "big.txt", u64::MAX)?;
    assert_eq!(full.statistics.blobs_diffed, 39);
    let bounded = super::blame_outcome(&gix, tip, "big.txt", size * 5)?;
    assert!(
        bounded.statistics.blobs_diffed <= 5,
        "diffed {} versions",
        bounded.statistics.blobs_diffed
    );
    let lines: u32 = bounded.entries.iter().map(|e| e.len.get()).sum();
    assert_eq!(lines, 4001);
    Ok(())
}

/// A file past the budget: once the horizon walk has found the horizon, the counting blame
/// fails its next version read however much budget is left, and a walk told the blame fit
/// gives up with no horizon. Red when a file past the cap pays a whole aborted blame pass
/// before the walk starts, or the walk keeps running after a blame that fit.
#[test]
fn the_counting_blame_stops_once_the_walk_finds_the_horizon() -> crate::Result<()> {
    let dir = tempfile::tempdir().map_err(crate::error::git)?;
    let p = dir.path();
    gitterrace_testkit::git_at(p, "2024-01-01T00:00:00Z", &["init", "-q", "-b", "main"]);
    let mut body = String::new();
    for n in 0..10 {
        body.push_str(&"x".repeat(40));
        body.push_str(&n.to_string());
        body.push('\n');
        std::fs::write(p.join("f.txt"), &body).map_err(crate::error::git)?;
        let date = format!("2024-01-01T00:0{n}:00Z");
        gitterrace_testkit::git_at(p, &date, &["add", "-A"]);
        gitterrace_testkit::git_at(p, &date, &["commit", "-qm", &format!("v{n}")]);
    }
    let repo = crate::Repo::discover(p)?;
    let gix = repo.repo();
    let tip = repo.resolve_id(&gix, "HEAD")?;
    let found = AtomicBool::new(true);
    let counted = super::Budgeted::new(&gix.objects, u64::MAX, &found);
    assert!(super::run_blame(&gix, &counted, tip, "f.txt", None).is_err());
    let stopped = super::history_horizon(&gix, tip, "f.txt", 500, &AtomicBool::new(true))?;
    assert_eq!((stopped.since, stopped.commits), (None, 0));
    let (bounded, walked) = super::blame_bounded(&gix, tip, "f.txt", 500)?;
    assert!(walked > 0);
    assert!(bounded.statistics.blobs_diffed < 9);
    Ok(())
}
