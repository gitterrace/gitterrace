//! One rule for unreadable caches: any cache this binary cannot read whole is `Unreadable`,
//! never partly served, never a panic, so a bad cache is rebuilt instead of shown. The store
//! cases run against the segment reader. The cases that rebuild through the index wait until
//! the index reads segments, and are ignored with that reason.
#![expect(clippy::expect_used, reason = "test setup")]

mod common;

use std::fs;
use std::path::Path;

use gitterrace_index::seg::SegError;
use gitterrace_index::seg::format::StringBase;
use gitterrace_index::seg::manifest::{MANIFEST, Manifest, Step};
use gitterrace_index::seg::read::Store;
use gitterrace_index::seg::strings::Interner;

use common::{entry, valid, write_store};

/// Opens and reads everything. `Ok(true)` read whole, `Ok(false)` no manifest.
fn open_all(dir: &Path) -> Result<bool, SegError> {
    match Store::open(dir)? {
        None => Ok(false),
        Some(s) => {
            s.check_all()?;
            gitterrace_index::seg::strings::Strings::new(&s.strings).check_all()?;
            Ok(true)
        }
    }
}

fn assert_unreadable(dir: &Path, case: &str) {
    match open_all(dir) {
        Err(e) if e.is_unreadable() => {}
        other => panic!("{case}: expected Unreadable, got {other:?}"),
    }
}

#[test]
fn a_valid_store_reads_whole() {
    let d = tempfile::tempdir().expect("tmp");
    valid(d.path());
    assert!(open_all(d.path()).expect("valid store"));
}

#[test]
fn a_missing_or_torn_manifest_is_unreadable_or_fresh() {
    let d = tempfile::tempdir().expect("tmp");
    valid(d.path());
    let m = fs::read(d.path().join(MANIFEST)).expect("manifest");
    for cut in [0, 1, m.len() / 2, m.len() - 1] {
        fs::write(d.path().join(MANIFEST), &m[..cut]).expect("tear");
        assert_unreadable(d.path(), &format!("manifest cut at {cut}"));
    }
    fs::remove_file(d.path().join(MANIFEST)).expect("rm");
    assert!(!open_all(d.path()).expect("no manifest is a fresh cache"));
}

#[test]
fn a_manifest_naming_a_missing_file_or_wrong_length_or_crc_is_unreadable() {
    for (case, edit) in [("missing", 0), ("length", 1), ("crc", 2)] {
        let d = tempfile::tempdir().expect("tmp");
        let mut m = valid(d.path());
        match edit {
            0 => fs::remove_file(d.path().join("seg-1-0.gts")).expect("rm"),
            1 => m.live[1].len += 1,
            _ => m.live[1].footer_crc ^= 1,
        }
        if edit != 0 {
            m.commit(d.path()).expect("rewrite manifest");
        }
        assert_unreadable(d.path(), case);
    }
}

/// Cut at 0, 1, half and len-1, plus one flipped byte at every offset of every live file:
/// all `Unreadable`, none read as valid, none panic.
#[test]
fn every_cut_and_every_flipped_byte_is_unreadable() {
    let d = tempfile::tempdir().expect("tmp");
    let m = valid(d.path());
    for f in &m.live {
        let path = d.path().join(&f.name);
        let good = fs::read(&path).expect("read");
        for cut in [0, 1, good.len() / 2, good.len() - 1] {
            fs::write(&path, &good[..cut]).expect("cut");
            assert_unreadable(d.path(), &format!("{} cut at {cut}", f.name));
        }
        for at in 0..good.len() {
            let mut bad = good.clone();
            bad[at] ^= 0x5a;
            fs::write(&path, &bad).expect("flip");
            assert_unreadable(d.path(), &format!("{} flipped at {at}", f.name));
        }
        fs::write(&path, &good).expect("restore");
    }
    assert!(open_all(d.path()).expect("restored"));
}

/// A crash after each step of the write sequence leaves either no manifest (a fresh cache, the
/// previous set was none) or the full new set, never a half one.
#[test]
fn a_crash_at_every_step_leaves_the_old_set_or_the_new_one() {
    let mut steps = Vec::new();
    {
        let d = tempfile::tempdir().expect("tmp");
        write_store(d.path(), &mut |s| {
            steps.push(s);
            true
        })
        .expect("write");
    }
    assert!(steps.contains(&Step::ManifestRenamed), "{steps:?}");
    for stop in 0..steps.len() {
        let d = tempfile::tempdir().expect("tmp");
        let mut n = 0;
        let r = write_store(d.path(), &mut |_| {
            n += 1;
            n <= stop
        });
        assert!(r.is_err(), "step {stop} did not stop the sequence");
        let renamed = steps[..=stop].contains(&Step::ManifestRenamed);
        match open_all(d.path()) {
            Ok(true) => assert!(
                renamed,
                "step {stop}: a set served before the manifest rename"
            ),
            Ok(false) => assert!(!renamed, "step {stop}: the renamed manifest was lost"),
            Err(e) => panic!("step {stop} ({:?}): {e}", steps[stop]),
        }
        // Whatever a crash left beside the live set is named by `strays`, so it can be swept.
        let m = Manifest::load(d.path()).expect("load");
        let strays = gitterrace_index::seg::manifest::strays(d.path(), m.as_ref()).expect("strays");
        for name in fs::read_dir(d.path()).expect("dir").flatten() {
            let name = name.file_name().to_string_lossy().into_owned();
            let live = m
                .as_ref()
                .is_some_and(|m| m.live.iter().any(|f| f.name == name));
            assert!(
                live || name == MANIFEST || strays.contains(&name),
                "step {stop}: {name} is neither live nor a stray"
            );
        }
    }
}

/// A string delta whose base is not the cumulative count of the deltas before it in
/// manifest order (deltas reordered, or one missing) is `Unreadable`.
#[test]
fn a_string_base_out_of_manifest_order_is_unreadable() {
    let d = tempfile::tempdir().expect("tmp");
    let mut m = valid(d.path());
    let mut second = Interner::new(StringBase {
        paths: 3,
        identities: 1,
        labels: 1,
    });
    second.path("src/c.rs").expect("path");
    let (e, _) = gitterrace_index::seg::strings::write_delta(
        d.path(),
        "str-2-0.gts",
        StringBase {
            paths: 3,
            identities: 1,
            labels: 1,
        },
        &second.into_delta(),
    )
    .expect("delta");
    // Reordered: the second delta before the first.
    let first = entry(&m, "str-1-0.gts").clone();
    m.live.retain(|f| f.name != "str-1-0.gts");
    m.live.insert(0, e);
    m.live.insert(1, first);
    m.commit(d.path()).expect("commit");
    assert_unreadable(d.path(), "reordered deltas");
    // Missing: only the second delta.
    m.live.retain(|f| f.name != "str-1-0.gts");
    m.commit(d.path()).expect("commit");
    assert_unreadable(d.path(), "missing first delta");
}

/// A path or identity over 65535 bytes is an error from the writer, never truncated: a cut
/// string would silently merge two paths.
#[test]
fn an_overlong_string_is_an_error_not_a_truncation() {
    let long = "a".repeat(65_536);
    let mut i = Interner::new(StringBase::default());
    assert!(matches!(i.path(&long), Err(SegError::TooLong { .. })));
    assert!(matches!(i.identity(&long), Err(SegError::TooLong { .. })));
    assert!(
        i.path(&"b".repeat(65_535)).is_ok(),
        "the limit itself is allowed"
    );
}

/// A `VERSION` file from today's JSON cache next to no manifest, or any older format, must be
/// deleted and rebuilt by the index, and the rebuilt files equal a fresh build byte for byte.
#[test]
#[ignore = "waits until the index reads and rebuilds the segment store"]
fn the_index_rebuilds_an_unreadable_cache_to_a_fresh_build() {
    panic!("not wired yet");
}

/// A newer `VERSION` stays untouched and the web shows the `newer_version` page, no 500 on any
/// `WARM_PAGES` path against any corrupted cache.
#[test]
#[ignore = "waits until the web pages read the segment store"]
fn a_newer_cache_is_left_alone_and_no_page_answers_500() {
    panic!("not wired yet");
}
