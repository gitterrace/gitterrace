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

/// A repository whose tree, written by plumbing so no disk folds it, holds `names` at the top.
fn names_repo(at: &Path, names: &[(&str, &str)]) -> Fixture {
    let f = Fixture::init(at);
    // Otherwise fast-import on macOS, where init sets it, folds the pair into one path.
    f.git(&["config", "core.ignorecase", "false"]);
    let mut s = String::from(
        "commit refs/heads/main\ncommitter T <t@example.com> 1700000000 +0000\ndata 2\nc\n",
    );
    for (name, body) in names {
        s += &[
            "M 100644 inline ",
            name,
            "\ndata ",
            &body.len().to_string(),
            "\n",
            body,
            "\n",
        ]
        .concat();
    }
    gitterrace_testkit::git_stdin(f.path(), &["fast-import", "--quiet"], s.as_bytes());
    f
}

/// Guard class: two names one disk stores as one file, by case or by Unicode form, and names
/// Windows reserves or strips. The later of a pair is skipped with a line naming both, the
/// first keeps its own page, and a reserved name is skipped like any hostile segment. Red
/// before the check: the second write replaced the first on APFS and NTFS.
#[test]
fn names_one_disk_stores_as_one_file_are_skipped_with_a_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    let nfd = "no\u{308}te.txt";
    let nfc = "n\u{f6}te.txt";
    let mut files = vec![
        ("README.md", "upper body"),
        ("readme.md", "lower body"),
        (nfd, "decomposed body"),
        (nfc, "composed body"),
        ("ok.txt", "plain body"),
    ];
    // Git for Windows refuses a device name in a commit, so only other systems can carry one.
    let device = cfg!(not(windows));
    if device {
        files.push(("NUL.txt", "device body"));
    }
    let src = names_repo(&home.join("src"), &files);
    let out = home.join("site");
    let o = export(home, &out, &[src.path()], &[]);
    let err = stderr(&o);
    assert!(o.status.success(), "{err}");
    // Names are reported quoted, a combining mark escaped.
    for [kept, skipped] in [["README.md", "readme.md"], [nfd, nfc]] {
        let [kept, skipped] = [format!("{kept:?}"), format!("{skipped:?}")];
        let line = err
            .lines()
            .find(|l| l.contains(&skipped))
            .unwrap_or_default();
        assert!(
            line.contains(&kept),
            "no line names {skipped} and {kept}: {err}"
        );
    }
    assert!(
        !device || err.contains("NUL.txt"),
        "the device name was not reported: {err}"
    );
    let blob = out.join("r/src/blob/HEAD");
    let names: Vec<String> = std::fs::read_dir(&blob)
        .expect("read")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    let readmes = names.iter().filter(|n| n.eq_ignore_ascii_case("readme.md"));
    assert_eq!(readmes.count(), 1, "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("NUL")), "{names:?}");
    let page = std::fs::read_to_string(blob.join("README.md/index.html")).expect("README page");
    assert!(page.contains("upper body") && !page.contains("lower body"));
    assert!(blob.join("ok.txt/index.html").is_file());
}

/// Every file under `dir` read as text, lossily, with its path.
fn texts(dir: &Path) -> Vec<(PathBuf, String)> {
    tree(dir)
        .into_iter()
        .map(|(p, b)| (p, String::from_utf8_lossy(&b).into_owned()))
        .collect()
}

/// Guard class: anything of this machine reaching a published export under the default
/// config: the working tree, a stash, a commit no public ref reaches, a ref outside heads,
/// tags and remotes, the declared actions or a host path. An export always follows the
/// public rule. Red when the export router took the rule from `server.is_public`, which is
/// off by default, so the crawl wrote the worktree and stash pages and the hidden ref.
#[test]
fn a_default_config_export_holds_nothing_a_public_server_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path();
    std::fs::create_dir_all(home.join("cfg")).expect("mkdir");
    std::fs::write(
        home.join("cfg/actions.toml"),
        "version = 1\n\n[[send]]\nid = \"leakysend\"\nlabel = \"Leaky send label\"\n\
         url = \"https://example.test/{title}\"\n\n[[action]]\nid = \"leakyaction\"\n\
         label = \"Leaky action label\"\nkinds = [\"commit\"]\ncommand = [\"true\"]\n",
    )
    .expect("actions.toml");
    let src = Fixture::standard(&home.join("src"))
        .write("hidden/side.txt", "unreachable body\n")
        .commit("chore: unreachable commit");
    let hidden = src.git(&["rev-parse", "HEAD"]).trim().to_owned();
    src.git(&["update-ref", "refs/secretref/leak", &hidden]);
    src.git(&["reset", "-q", "--hard", "HEAD~1"]);
    src.git(&["reflog", "expire", "--expire=now", "--all"]);
    let src = src
        .write("src/a.rs", "stashed body\n")
        .stash("stashed message")
        .write("notes/draft.txt", "uncommitted body\n");
    let out = home.join("site");
    let o = export(home, &out, &[src.path()], &[]);
    assert!(o.status.success(), "{}", stderr(&o));
    let home_path = home.to_string_lossy().into_owned();
    let banned = [
        "uncommitted body",
        "stashed body",
        "stashed message",
        "unreachable body",
        "unreachable commit",
        hidden.as_str(),
        &hidden[..12],
        "secretref",
        "leakysend",
        "Leaky send label",
        "leakyaction",
        "Leaky action label",
        home_path.as_str(),
    ];
    let mut found = Vec::new();
    for (path, body) in texts(&out) {
        for b in banned {
            if body.contains(b) || path.to_string_lossy().contains(b) {
                found.push(format!("{} holds {b:?}", path.display()));
            }
        }
        let p = path.to_string_lossy();
        for local in [
            "worktree",
            "stashes",
            "_settings",
            "_actions",
            "_send",
            "events",
        ] {
            if p.split('/').any(|seg| seg == local) {
                found.push(format!("{p} is a local page"));
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}
