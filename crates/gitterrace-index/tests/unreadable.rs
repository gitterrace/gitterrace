//! One rule for unreadable caches: any cache this binary cannot read whole is `Unreadable`,
//! never partly served, never a panic, so a bad cache is rebuilt instead of shown. The store
//! cases run against the segment reader, the rebuild and newer-cache cases through the index.
//! The pages over a newer cache are checked in the web crate's `newer_cache` test.
#![expect(clippy::expect_used, reason = "test setup")]

mod common;

use std::fs;
use std::path::Path;

use gitterrace_index::seg::SegError;
use gitterrace_index::seg::format::{StringBase, section};
use gitterrace_index::seg::manifest::{MANIFEST, Manifest, Step};
use gitterrace_index::seg::read::Store;
use gitterrace_index::seg::strings::Interner;

use common::{entry, valid, write_store};

/// Opens and reads everything. `Ok(true)` read whole, `Ok(false)` no manifest.
/// A named way to corrupt a cache directory.
type Case<'a> = (&'a str, &'a dyn Fn(&Path));

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

/// A small repository and its HEAD id, for the cases that build through the index.
fn repo(dir: &Path) -> (gitterrace_git::Repo, String) {
    let mut f = gitterrace_testkit::Fixture::init(&dir.join("repo")).at("2024-01-01T00:00:00Z");
    for i in 0..6 {
        f = f
            .write(&format!("src/f{}.rs", i % 3), &format!("edit {i}\n"))
            .commit(&format!("feat: step {i}"));
    }
    let head = f.git(&["rev-parse", "HEAD"]);
    (
        gitterrace_git::Repo::discover(f.path()).expect("open"),
        head,
    )
}

/// Every file in the cache folder except the lock, by name.
fn folder(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "lock")
        .map(|n| {
            let b = fs::read(dir.join(&n)).expect("read");
            (n, b)
        })
        .collect()
}

fn build(dir: &Path, repo: &gitterrace_git::Repo, head: &str) -> gitterrace_index::Index {
    let index = gitterrace_index::Index::open_dir(dir.to_owned()).expect("open");
    index.update(repo, head, &|_| {}).expect("update");
    index
}

/// Old `VERSION` files, a torn manifest, a cut segment, a flipped byte inside a segment block
/// and a manifest naming a missing file: opening again never serves the bad set. Either the
/// open wipes it, or the set fails its block check (the page-time path) and is rebuilt. Each
/// time the rebuilt folder equals a fresh build byte for byte.
#[test]
fn the_index_rebuilds_an_unreadable_cache_to_a_fresh_build() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (repo, head) = repo(tmp.path());
    let fresh_dir = tmp.path().join("fresh");
    build(&fresh_dir, &repo, &head);
    let fresh = folder(&fresh_dir);
    let m = Manifest::load(&fresh_dir).expect("load").expect("a set");
    let seg = m
        .live
        .iter()
        .find(|f| f.name.starts_with("seg-"))
        .expect("a segment")
        .name
        .clone();
    let store = Store::open(&fresh_dir).expect("open").expect("set");
    let block = store.segments[0].blocks(section::COMMITS)[0];
    let in_block = usize::try_from(block.offset).expect("offset") + 1;
    drop(store);

    let cases: [Case<'_>; 7] = [
        ("pre-release VERSION", &|d| {
            fs::write(d.join("VERSION"), "9\n").expect("w");
        }),
        ("VERSION 0 beside the old record files", &|d| {
            fs::write(d.join("VERSION"), "0\n").expect("w");
            fs::write(d.join("commits.jsonl"), "{\"id\":\"x\"}\n").expect("w");
        }),
        ("garbage VERSION", &|d| {
            fs::write(d.join("VERSION"), "v?\n").expect("w");
        }),
        ("torn manifest", &|d| {
            let b = fs::read(d.join(MANIFEST)).expect("r");
            fs::write(d.join(MANIFEST), &b[..b.len() / 2]).expect("w");
        }),
        ("segment cut in half", &|d| {
            let b = fs::read(d.join(&seg)).expect("r");
            fs::write(d.join(&seg), &b[..b.len() / 2]).expect("w");
        }),
        ("byte flipped in the commits block", &|d| {
            let mut b = fs::read(d.join(&seg)).expect("r");
            b[in_block] ^= 0x5a;
            fs::write(d.join(&seg), &b).expect("w");
        }),
        ("named segment missing", &|d| {
            fs::remove_file(d.join(&seg)).expect("rm");
        }),
    ];
    for (case, corrupt) in cases {
        let dir = tmp.path().join("cache");
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("clean");
        }
        build(&dir, &repo, &head);
        corrupt(&dir);
        let index = gitterrace_index::Index::open_dir(dir.clone()).expect("reopen");
        assert_eq!(
            index.newer_format(),
            None,
            "{case}: taken for a newer cache"
        );
        // What survived the open must read whole, or be rebuilt: never served in part.
        match open_all(&dir) {
            Ok(_) => {}
            Err(e) if e.is_unreadable() => index.rebuild().expect("rebuild"),
            Err(e) => panic!("{case}: {e}"),
        }
        index.update(&repo, &head, &|_| {}).expect("update");
        let got = folder(&dir);
        assert_eq!(
            got.keys().collect::<Vec<_>>(),
            fresh.keys().collect::<Vec<_>>(),
            "{case}: files after the rebuild"
        );
        for (name, body) in &fresh {
            assert!(
                got[name] == *body,
                "{case}: {name} differs from a fresh build"
            );
        }
    }
}

/// A `VERSION` newer than this build's belongs to a newer binary: open, update and rebuild
/// leave every byte of it alone, and the index says why instead of reading it.
#[test]
fn a_newer_cache_is_left_alone_and_no_page_answers_500() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (repo, head) = repo(tmp.path());
    let dir = tmp.path().join("cache");
    build(&dir, &repo, &head);
    let newer = gitterrace_index::FORMAT_VERSION
        .parse::<u64>()
        .expect("numeric")
        + 1;
    fs::write(dir.join("VERSION"), format!("v{newer}\n")).expect("w");
    // A file this build does not know, as a newer format might add.
    fs::write(dir.join("future.gts"), b"from a newer build").expect("w");
    // Even a corrupt set: a newer build's bytes are not ours to judge.
    let m = Manifest::load(&dir).expect("load").expect("a set");
    let seg = m
        .live
        .iter()
        .find(|f| f.name.starts_with("seg-"))
        .expect("seg");
    let mut b = fs::read(dir.join(&seg.name)).expect("r");
    let last = b.len() - 1;
    b[last] ^= 1;
    fs::write(dir.join(&seg.name), &b).expect("w");
    let before = folder(&dir);

    let index = gitterrace_index::Index::open_dir(dir.clone()).expect("open");
    assert_eq!(index.newer_format(), Some(newer.to_string().as_str()));
    let e = index
        .update(&repo, &head, &|_| {})
        .expect_err("update must refuse");
    assert!(e.to_string().contains("newer"), "{e}");
    assert!(
        index.rebuild().is_err(),
        "rebuild must refuse a newer cache"
    );
    assert!(index.records(&repo, &head).is_err(), "records must refuse");
    drop(index);
    let again = gitterrace_index::Index::open_dir(dir.clone()).expect("open");
    assert!(again.newer_format().is_some());
    assert!(folder(&dir) == before, "the newer cache was changed");
}
