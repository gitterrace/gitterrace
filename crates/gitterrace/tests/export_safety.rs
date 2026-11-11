//! `export` through the real binary against disposable folders: it never removes, moves or
//! writes through anything it does not own. Every case checks the exit status and that the
//! source repositories and unrelated files survive byte for byte.
#![expect(clippy::expect_used, reason = "test setup")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use gitterrace_testkit::Fixture;

/// Runs `git-terrace export <out> <repos...> [extra...]` with its config and cache in `home`.
fn export(home: &Path, out: &Path, repos: &[&Path], extra: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_git-terrace"));
    cmd.arg("export")
        .arg(out)
        .args(repos)
        .args(extra)
        .arg("--quiet")
        .arg("--config")
        .arg(home.join("cfg"))
        .env("HOME", home)
        .env("XDG_CACHE_HOME", home.join("cache"));
    cmd.output().expect("run git-terrace")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// Every file under `dir` (links as their target text), `.git` included, so a removed or
/// rewritten byte anywhere shows up. The export's own cache folder is left out.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).expect("read_dir") {
            let p = e.expect("entry").path();
            let rel = p.strip_prefix(dir).expect("under dir").to_path_buf();
            if rel.ends_with("gitterrace") || rel.ends_with("info/exclude") {
                continue;
            }
            let m = std::fs::symlink_metadata(&p).expect("lstat");
            if m.is_symlink() {
                let to = std::fs::read_link(&p).expect("readlink");
                out.insert(rel, to.to_string_lossy().as_bytes().to_vec());
            } else if m.is_dir() {
                todo.push(p);
            } else {
                out.insert(rel, std::fs::read(&p).expect("read"));
            }
        }
    }
    out
}

fn repo(at: &Path) -> Fixture {
    Fixture::standard(at).write("notes/kept.txt", "uncommitted\n")
}

/// Guard class: an output folder that holds a served repository, whether named directly, as
/// an ancestor, forced, or among several repositories. Each is refused before anything is
/// written and every repository stays as it was. Red when the overlap check runs only for an
/// earlier export's folder: a forced merge then replaced the repository's own entries.
#[test]
fn an_output_holding_a_served_repository_is_refused_even_when_forced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let site = home.join("site");
    let inner = repo(&site.join("r"));
    let other = repo(&home.join("other"));
    std::fs::write(site.join("mine.txt"), "keep").expect("write");
    let before = tree(home);
    let cases: [(&Path, Vec<&Path>, &[&str]); 4] = [
        (&site, vec![inner.path()], &["--force"]),
        (&site, vec![other.path(), inner.path()], &["--force"]),
        (&site, vec![other.path(), inner.path()], &[]),
        (home, vec![other.path()], &["--force"]),
    ];
    for (out, repos, extra) in cases {
        let o = export(home, out, &repos, extra);
        assert!(
            !o.status.success(),
            "{} {repos:?} was accepted",
            out.display()
        );
        assert!(stderr(&o).contains("holds"), "{}", stderr(&o));
        assert_eq!(
            tree(home),
            before,
            "export into {} changed files",
            out.display()
        );
    }
}

/// Guard class: an export that takes over a folder it did not make. A forced export into a
/// foreign folder and every export after it keep each unrelated file, nested ones too, and
/// the later ones need no `--force`. Red when the marker the first export writes makes the
/// whole folder look like an earlier export's, which the next one then cleared.
#[test]
fn repeated_exports_into_a_foreign_folder_keep_every_unrelated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let src = repo(&home.join("src"));
    let out = home.join("shared");
    std::fs::create_dir_all(out.join("docs/deep")).expect("mkdir");
    std::fs::write(out.join("docs/deep/plan.txt"), "plan").expect("write");
    std::fs::write(out.join("top.txt"), "top").expect("write");
    let refused = export(home, &out, &[src.path()], &[]);
    assert!(
        !refused.status.success(),
        "a foreign folder was accepted without --force"
    );
    let forced = export(home, &out, &[src.path()], &["--force"]);
    assert!(forced.status.success(), "{}", stderr(&forced));
    let source = tree(src.path());
    for run in 0..2 {
        let o = export(home, &out, &[src.path()], &[]);
        assert!(o.status.success(), "run {run}: {}", stderr(&o));
        assert_eq!(std::fs::read(out.join("top.txt")).expect("top"), b"top");
        assert_eq!(
            std::fs::read(out.join("docs/deep/plan.txt")).expect("nested"),
            b"plan"
        );
        assert!(out.join("index.html").is_file(), "run {run}: no site");
        assert_eq!(tree(src.path()), source, "run {run}: the source changed");
    }
}

/// A forced export over a foreign folder replaces a colliding file, which is what `--force`
/// asks for, but refuses to clear a colliding folder of someone else's: that would erase a
/// tree the export does not own. Nothing is written when it refuses.
#[test]
fn a_forced_export_never_clears_a_foreign_folder_in_its_way() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let a = repo(&home.join("alpha"));
    let b = repo(&home.join("beta"));
    let out = home.join("shared");
    std::fs::create_dir_all(out.join("r/alpha")).expect("mkdir");
    std::fs::write(out.join("r/alpha/mine.txt"), "mine").expect("write");
    let before = tree(home);
    let o = export(home, &out, &[a.path(), b.path()], &["--force"]);
    assert!(!o.status.success(), "a foreign folder was cleared");
    assert!(
        stderr(&o).contains("not made by an export"),
        "{}",
        stderr(&o)
    );
    assert_eq!(tree(home), before);
}

/// A normal update of an earlier export removes the pages the new site lacks and
/// leaves the folder beside it and the source alone.
#[test]
fn a_managed_update_drops_stale_pages_and_nothing_else() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let src = repo(&home.join("src"));
    let out = home.join("site");
    std::fs::write(home.join("beside.txt"), "beside").expect("write");
    assert!(export(home, &out, &[src.path()], &[]).status.success());
    std::fs::create_dir_all(out.join("old/page")).expect("mkdir");
    std::fs::write(out.join("old/page/index.html"), "stale").expect("write");
    let source = tree(src.path());
    let o = export(home, &out, &[src.path()], &[]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!out.join("old").exists(), "a stale page survived");
    assert!(out.join("index.html").is_file());
    assert_eq!(
        std::fs::read(home.join("beside.txt")).expect("beside"),
        b"beside"
    );
    assert_eq!(tree(src.path()), source);
}

/// Guard class: an output path that leads somewhere else. A symlink as the output is refused,
/// forced or not, and nothing is written through it; a symlinked parent that makes the
/// output an ancestor of the repository is seen through. Red when the link is followed.
#[cfg(unix)]
#[test]
fn a_symlinked_output_or_alias_is_refused_and_never_written_through() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let src = repo(&home.join("base/site/r"));
    let ext = home.join("ext");
    std::fs::create_dir(&ext).expect("mkdir");
    let link = home.join("out");
    std::os::unix::fs::symlink(&ext, &link).expect("symlink");
    let alias = home.join("alias");
    std::os::unix::fs::symlink(home.join("base"), &alias).expect("symlink");
    let before = tree(home);
    for extra in [&[][..], &["--force"]] {
        let o = export(home, &link, &[src.path()], extra);
        assert!(
            !o.status.success(),
            "a symlinked output was written through"
        );
        assert!(stderr(&o).contains("symlink"), "{}", stderr(&o));
        let o = export(home, &alias.join("site"), &[src.path()], extra);
        assert!(
            !o.status.success(),
            "an aliased ancestor of the repository was accepted"
        );
        assert_eq!(tree(home), before, "{extra:?} changed files");
    }
    assert_eq!(std::fs::read_dir(&ext).expect("read").count(), 0);
}

/// A pid no process has: a child that has exited and been reaped.
#[cfg(unix)]
fn dead_pid() -> u32 {
    let mut child = Command::new("true").spawn().expect("spawn");
    let pid = child.id();
    child.wait().expect("wait");
    pid
}

/// Guard class: an earlier site lost to an interrupted replacement. A crash between the two
/// renames leaves the old site aside and nothing at the output; the next export puts it back
/// before anything else, so a failing export leaves it in place and a succeeding one replaces
/// it. Red when the cleanup of crashed runs deletes the only surviving site.
#[cfg(unix)]
#[test]
fn a_site_left_aside_by_a_crash_is_restored_by_the_next_export() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let src = repo(&home.join("src"));
    let out = home.join("site");
    assert!(export(home, &out, &[src.path()], &[]).status.success());
    let site = tree(&out);
    let aside = home.join(format!(".site.partial-{}.old", dead_pid()));
    std::fs::rename(&out, &aside).expect("simulate the crash");
    let failed = export(home, &out, &[src.path()], &["--export-pages", "1"]);
    assert!(!failed.status.success(), "the capped export passed");
    assert!(!aside.exists(), "the old site was left aside");
    assert_eq!(tree(&out), site, "the old site was not restored whole");
    std::fs::rename(&out, &aside).expect("simulate the crash again");
    let o = export(home, &out, &[src.path()], &[]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(out.join("index.html").is_file());
    assert!(
        !aside.exists(),
        "the old site stayed after a new one was in place"
    );
    let left: Vec<_> = std::fs::read_dir(home)
        .expect("read")
        .map(|e| e.expect("entry").file_name())
        .filter(|n| n.to_string_lossy().contains("partial"))
        .collect();
    assert!(left.is_empty(), "left beside the site: {left:?}");
}
