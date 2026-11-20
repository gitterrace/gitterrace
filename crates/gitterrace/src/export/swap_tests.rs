use std::path::Path;

use crate::export::out::{MARKER, Owned, Target, marker};
use crate::export::swap::{Dir, Staging, aside_name, partial_prefix, recover_with};

fn target(dir: &Path, name: &str) -> Target {
    Target::resolve(&dir.join(name)).expect("resolve")
}

/// A whole site at `path`, as an export leaves it.
fn site(path: &Path, page: &str) {
    std::fs::create_dir_all(path).expect("mkdir");
    std::fs::write(path.join("index.html"), page).expect("write");
    std::fs::write(path.join(MARKER), marker(&[], &Owned::All)).expect("marker");
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .expect("read")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Guard class: an export that deletes a folder it did not make. Beside the target, only
/// staging folders of this target whose process is gone are removed: a live run's folder,
/// another target's, an unparsable pid, a plain file and a symlink by a stale name all stay.
/// Red when the cleanup matches loosely or ignores the pid.
#[test]
fn only_dead_staging_folders_of_this_target_are_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let mk = |n: &str| std::fs::create_dir(p.join(n)).expect("mkdir");
    for n in [".site.partial-111", ".site.partial-222"] {
        mk(n);
        std::fs::write(p.join(n).join("page"), "x").expect("write");
    }
    for n in [".other.partial-111", ".site.partial-11x", ".site.partial-0111", "site"] {
        mk(n);
    }
    std::fs::write(p.join(".site.partial-333"), "a file").expect("write");
    mk("outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(p.join("outside"), p.join(".site.partial-444")).expect("symlink");
    recover_with(&target(p, "site"), |pid| pid == 222).expect("cleanup");
    let mut want = vec![
        ".other.partial-111",
        ".site.partial-0111",
        ".site.partial-11x",
        ".site.partial-222",
        ".site.partial-333",
        "outside",
        "site",
    ];
    if cfg!(unix) {
        want.push(".site.partial-444");
        want.sort_unstable();
    }
    assert_eq!(names(p), want);
    assert!(p.join(".site.partial-222/page").is_file());
}

/// Crash recovery of the swap: a site set aside by a run that is gone comes back when nothing
/// took its place, and is left alone while its run is alive. Red when the cleanup of crashed
/// runs deletes the only surviving site.
#[test]
fn a_site_left_aside_by_a_crashed_swap_is_restored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let old = p.join(format!("{}111.old", partial_prefix("site")));
    site(&old, "old site");
    recover_with(&target(p, "site"), |_| true).expect("cleanup while alive");
    assert!(old.is_dir(), "a running export's old site was touched");
    recover_with(&target(p, "site"), |_| false).expect("recover after the crash");
    assert!(!old.exists(), "the old site stayed aside");
    let page = std::fs::read_to_string(p.join("site/index.html")).expect("restored");
    assert_eq!(page, "old site");
}

/// Once a whole new site is in place the copy set aside is removed; a folder at the output
/// that is not an export's own, or an aside folder with no marker, keeps it.
#[test]
fn a_site_set_aside_is_removed_only_when_a_new_site_is_in_place() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let old = p.join(".site.partial-111.old");
    site(&old, "old");
    site(&p.join("site"), "new");
    recover_with(&target(p, "site"), |_| false).expect("recover");
    assert!(!old.exists(), "the replaced site stayed");
    assert_eq!(std::fs::read(p.join("site/index.html")).expect("new"), b"new");
    site(&old, "old");
    std::fs::remove_file(p.join("site").join(MARKER)).expect("unmark");
    recover_with(&target(p, "site"), |_| false).expect("recover");
    assert!(old.is_dir(), "an old site was dropped for a folder that is not one");
    let bare = p.join(".site.partial-222.old");
    std::fs::create_dir(&bare).expect("mkdir");
    std::fs::write(bare.join("mine"), "x").expect("write");
    recover_with(&target(p, "other"), |_| false).expect("recover");
    recover_with(&target(p, "site"), |_| false).expect("recover");
    assert!(bare.join("mine").is_file(), "an unmarked folder was touched");
}

/// Guard class: a failed publication that loses the old site. When the second rename fails,
/// the old site is moved back and nothing is left aside. Red when the old site is only
/// moved aside and the error returned.
#[test]
fn a_failed_second_rename_puts_the_old_site_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    site(&p.join("site"), "old");
    let t = target(p, "site");
    let staging = Staging::new(&t).expect("staging");
    // Gone from under it after the site was written, it cannot be renamed into place.
    std::fs::remove_dir_all(staging.path()).expect("break the staging folder");
    let parent = Dir::open(p).expect("parent");
    let err = staging.replace(&parent).expect_err("replace");
    assert!(format!("{err:#}").contains("moving"), "{err:#}");
    assert_eq!(std::fs::read(p.join("site/index.html")).expect("old"), b"old");
    assert!(!p.join(aside_name("site")).exists());
    assert_eq!(names(p), vec!["site"]);
}

/// A shared folder: the export's own entries are replaced, the ones the new site lacks go,
/// and the rest of the folder stays, a folder of someone else's included.
#[test]
fn a_shared_folder_keeps_what_the_export_does_not_own() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let out = p.join("site");
    std::fs::create_dir_all(out.join("mine")).expect("mkdir");
    std::fs::write(out.join("mine/a"), "a").expect("write");
    std::fs::write(out.join("stale.html"), "stale").expect("write");
    std::fs::write(out.join("index.html"), "old").expect("write");
    let prev = Owned::Entries(["index.html", "stale.html"].map(String::from).into());
    let t = target(p, "site");
    let staging = Staging::new(&t).expect("staging");
    std::fs::write(staging.path().join("index.html"), "new").expect("page");
    staging.publish(&[], Some(prev), false).expect("publish");
    assert_eq!(names(&out), vec![MARKER, "index.html", "mine"]);
    assert_eq!(std::fs::read(out.join("index.html")).expect("page"), b"new");
    assert_eq!(std::fs::read(out.join("mine/a")).expect("kept"), b"a");
    let marked = std::fs::read_to_string(out.join(MARKER)).expect("marker");
    assert!(marked.ends_with("\nowns index.html\n"), "{marked}");
    assert_eq!(names(p), vec!["site"]);
}
