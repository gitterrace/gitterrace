//! Idle memory of a public server after a full commit id: the reachable set that answers it.

use std::io::Write;

use gitterrace_testkit::synthetic::{self, Shape};

use crate::{MB, Server, apply_and_assert_allocator, serve_if_child};

/// Live heap the release may keep beyond the same page by tag. Half the 2 MB the set holds.
const HEAP_SLACK_MB: u64 = 1;

/// A full id on a public server walks every commit a public ref reaches (100k here) into a
/// set. The idle release drops it with the other caches, so an idle server holds no
/// per-commit state for it.
#[test]
fn public_full_id_set_goes_with_the_idle_release() {
    if serve_if_child() {
        return;
    }
    apply_and_assert_allocator();
    if cfg!(not(any(target_os = "macos", target_os = "linux"))) {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    synthetic::build(dir.path(), Shape::sized(100_000));
    let root = gitterrace_testkit::git(dir.path(), &["rev-list", "--max-parents=0", "HEAD"]);
    gitterrace_testkit::git(dir.path(), &["tag", "root", root.trim()]);
    // Not a root itself, so its full id walks the history.
    let child = gitterrace_testkit::git(
        dir.path(),
        &["rev-list", "--reverse", "--ancestry-path", &format!("{}..HEAD", root.trim())],
    );
    let child = child.lines().next().expect("a second commit").to_owned();
    let s = Server::start_with(
        dir.path(),
        "public::public_full_id_set_goes_with_the_idle_release",
        true,
    );
    let base = s.base();
    s.wait_ready(&base);
    s.wait_released(&base, 0);
    // The same page by tag first: what the page itself leaves behind (it holds no cache, so no
    // release follows) is the baseline, and only the set the full id builds is measured.
    assert_eq!(s.get(&format!("{base}/commit/root")).0, 200);
    let before = s.heap();
    let released = s.releases(&base);
    let path = format!("{base}/commit/{child}");
    assert_eq!(s.get(&path).0, 200, "{path}");
    s.wait_released(&base, released);
    let after = s.heap();
    writeln!(
        std::io::stderr(),
        "public full id: live heap {} KB before, {} KB after the release",
        before / 1024,
        after / 1024
    )
    .expect("stderr");
    assert!(
        after <= before + HEAP_SLACK_MB * MB,
        "the release keeps {} KB of the full-id set",
        after.saturating_sub(before) / 1024
    );
}
