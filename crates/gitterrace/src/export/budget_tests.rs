//! Guards on what an export costs: its page budget, its time per page and what a failed run
//! leaves behind.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gitterrace_testkit::Fixture;

use crate::export::{Scope, export_capped, fetch};

/// Files a commit page shows in full before the rest fold (`TOTAL_DIFF_FILES` in the web
/// crate). Every file in these fixtures is small, so only this budget folds them.
const SHOWN: usize = 50;

fn scope(commits: usize) -> Scope {
    Scope {
        commits,
        max_file: 64 * 1024,
        projects: Vec::new(),
        force: false,
    }
}

/// `commits` commits after a root, each changing `width` files.
fn wide(dir: &Path, commits: usize, width: usize) -> Fixture {
    let mut f = Fixture::init(dir);
    for c in 0..=commits {
        for i in 0..width {
            f = f.write(&format!("w/{i:03}.txt"), &format!("commit {c}\nfile {i}\n"));
        }
        f = f.commit(&format!("wide {c}"));
    }
    f
}

/// Every file under `dir` with its bytes, and every symlink with its target.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read_dir") {
            let p = e.expect("entry").path();
            let kind = std::fs::symlink_metadata(&p).expect("meta").file_type();
            let rel = p.strip_prefix(dir).expect("under dir").to_path_buf();
            if kind.is_symlink() {
                let to = std::fs::read_link(&p).expect("link");
                out.insert(rel, to.to_string_lossy().into_owned().into_bytes());
            } else if kind.is_dir() {
                stack.push(p);
            } else {
                out.insert(rel, std::fs::read(&p).expect("read"));
            }
        }
    }
    out
}

/// Fragment pages written, as `(commit, index)`.
fn fragments(out: &Path) -> BTreeSet<(String, usize)> {
    tree(out)
        .into_keys()
        .filter_map(|p| {
            let parts: Vec<String> = p
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let at = parts.iter().position(|s| s == "diff")?;
            Some((parts.get(at + 1)?.clone(), parts.get(at + 2)?.parse().ok()?))
        })
        .collect()
}

/// Guard class: an export whose size follows commit width instead of its scope. On commits
/// that each fold many files, the export writes exactly the fragments of the commits in scope
/// (none of an older commit), and every one of them counts against the one page budget: the
/// smallest budget that fits is the number of pages written, one less fails. Red when
/// fragments are counted apart from pages, skipped by the budget, or leak past the scope.
#[tokio::test]
async fn a_wide_export_writes_the_expected_pages_within_one_budget() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (commits, width) = (4, SHOWN + 12);
    let fx = wide(&dir.path().join("r"), commits, width);
    let repos = [fx.path().to_path_buf()];
    let in_scope = commits - 1;
    let repo = gitterrace_git::Repo::discover(fx.path()).expect("repo");
    let head = repo.resolve("HEAD").expect("head");
    let mut want = BTreeSet::new();
    for c in repo.log(&head, None, 0, in_scope).expect("log") {
        let files = repo.commit(&c.id).expect("commit").files.len();
        assert_eq!(files, width, "fixture commit {}", c.id);
        want.extend((SHOWN..files).map(|n| (c.id.clone(), n)));
    }
    // The crawl only follows links, and no in-scope page links an older commit's files, so
    // the export alone cannot show the scope check. Ask the real scope for every commit.
    let allowed = crate::export::Allowed::new(&repo, &head, &scope(in_scope)).expect("scope");
    let all = repo.log(&head, None, 0, usize::MAX).expect("log");
    assert_eq!(all.len(), commits + 1, "fixture history");
    for (i, c) in all.iter().enumerate() {
        for n in [0, SHOWN, width - 1] {
            let link = format!("/diff/{}/{n}", c.id);
            assert_eq!(allowed.contains(&link), i < in_scope, "{link} (commit {i})");
        }
    }
    let out = dir.path().join("out");
    let n = export_capped(&repos, &out, &scope(in_scope), usize::MAX)
        .await
        .expect("export");
    assert_eq!(fragments(&out), want, "fragment pages");
    // Every file but the forward page, the marker and the two fixed files took a page.
    let pages = tree(&out).len() - 4;
    assert_eq!(n, pages + 2, "files reported");
    assert!(
        pages > want.len(),
        "{pages} pages, {} fragments",
        want.len()
    );

    let fits = dir.path().join("fits");
    export_capped(&repos, &fits, &scope(in_scope), pages)
        .await
        .expect("the exact budget fits");
    assert_eq!(fragments(&fits), want);
    let over = dir.path().join("over");
    let err = export_capped(&repos, &over, &scope(in_scope), pages - 1)
        .await
        .expect_err("one page short must fail, fragments included");
    assert!(err.to_string().contains("cap of"), "{err}");
    assert!(!over.exists(), "a failed export left {}", over.display());
}

/// Guard class: a page whose cost grows with its commit instead of with itself. A commit that
/// replaces 120 files with 120 others (rename detection compares every pair, as in a large
/// rollup) is rendered once as its commit page; then all its folded files are fetched. Their
/// total time must stay within ten commit pages. Red when a folded file costs anything like
/// its whole commit again (a redone tree diff, a reread of every blob): 170 fragments at that
/// cost are seventeen times the budget.
#[tokio::test]
async fn folded_files_cost_less_than_ten_commit_pages() {
    let dir = tempfile::tempdir().expect("tempdir");
    let width = 120;
    let mut fx = Fixture::init(&dir.path().join("r"));
    for i in 0..width {
        fx = fx.write(
            &format!("old/{i:03}.txt"),
            &format!("old {i}\n{}", "a\n".repeat(i)),
        );
    }
    fx = fx.commit("root");
    std::fs::remove_dir_all(fx.path().join("old")).expect("rm");
    for i in 0..width {
        fx = fx.write(
            &format!("new/{i:03}.txt"),
            &format!("new {i}\n{}", "b\n".repeat(i)),
        );
    }
    fx = fx.commit("replace everything");
    let repo = gitterrace_git::Repo::discover(fx.path()).expect("repo");
    let id = repo.resolve("HEAD").expect("head");
    // The router an export builds, with the memo its run owns.
    let changes = std::sync::Arc::new(gitterrace_git::ChangeMemo::new());
    let router =
        gitterrace_web::export_router(vec![fx.path().to_path_buf()], &changes).expect("router");
    let site = format!("/r/{}", repo.name());

    let t = Instant::now();
    let (status, _, body) = fetch(&router, &format!("{site}/commit/{id}"))
        .await
        .expect("page");
    let page = t.elapsed();
    assert!(status.is_success(), "{status}");
    let links: Vec<String> = crate::export::links(&String::from_utf8_lossy(&body))
        .into_iter()
        .filter(|l| l.contains("/diff/"))
        .collect();
    assert!(
        links.len() >= 2 * width - SHOWN,
        "{} folded files",
        links.len()
    );
    let budget = page * 10 + Duration::from_millis(50);
    let t = Instant::now();
    for (done, link) in links.iter().enumerate() {
        let (status, _, _) = fetch(&router, link).await.expect("fragment");
        assert!(status.is_success(), "{link}: {status}");
        // Stop as soon as the budget is gone, so a regression fails fast instead of slowly.
        assert!(
            t.elapsed() <= budget,
            "{} of {} folded files took {:?}, over ten commit pages ({page:?} each)",
            done + 1,
            links.len(),
            t.elapsed()
        );
    }
}

/// Guard class: an export that fails part way and leaves its folder changed. An earlier
/// export and a forced foreign folder both stay byte-identical when the next export hits its
/// page cap, nothing is left beside them, and the error names the cap and the flags that
/// lower it. Red when an export clears or writes into its folder before it has finished.
#[tokio::test]
async fn a_capped_export_leaves_the_existing_folder_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fx = Fixture::standard(&dir.path().join("r"));
    let repos = [fx.path().to_path_buf()];
    let parent = dir.path().join("sites");
    std::fs::create_dir(&parent).expect("mkdir");
    let earlier = parent.join("earlier");
    export_capped(&repos, &earlier, &scope(2), usize::MAX)
        .await
        .expect("first export");
    let foreign = parent.join("foreign");
    std::fs::create_dir(&foreign).expect("mkdir");
    std::fs::write(foreign.join("mine.txt"), "keep").expect("write");
    #[cfg(unix)]
    std::os::unix::fs::symlink("mine.txt", foreign.join("link")).expect("symlink");
    for (out, force) in [(&earlier, false), (&foreign, true)] {
        let before = tree(out);
        let capped = Scope { force, ..scope(2) };
        let err = export_capped(&repos, out, &capped, 3)
            .await
            .expect_err("capped");
        let msg = err.to_string();
        for part in [
            "cap of 3 pages",
            "--commits (now 2)",
            "--max-file-kb (now 64)",
        ] {
            assert!(msg.contains(part), "{part:?} missing from: {msg}");
        }
        assert_eq!(tree(out), before, "{} changed", out.display());
    }
    let left: Vec<_> = std::fs::read_dir(&parent)
        .expect("read")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(left.len(), 2, "left beside the sites: {left:?}");
}
