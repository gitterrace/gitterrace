//! Hostile bytes: the cache sits in a `.git/` someone else may have
//! prepared, so the reader trusts no field. A seeded xorshift loop feeds random files, valid
//! containers with random flips, truncations and extensions, and size fields set to
//! `u32::MAX` / `u64::MAX` with every crc recomputed so the bound check itself is reached.
//! Each must be `Unreadable` or a correct read: never a panic, never an `Io` error from a
//! huge buffer.
//!
//! Every read runs under a counting global allocator (`cap`) whose limit is the bytes live
//! before the read plus 2 MiB: an allocation sized by a hostile field fails and aborts the
//! test binary. Tests here take one lock, so no other thread allocates under that limit.
#![expect(clippy::expect_used, reason = "test setup")]

mod common;

use std::alloc::System;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use cap::Cap;

use gitterrace_index::seg::SegError;
use gitterrace_index::seg::manifest::MANIFEST;
use gitterrace_index::seg::read::Store;
use gitterrace_testkit::synthetic::Rng;

const ROUNDS: usize = 3000;
const MIB: usize = 1024 * 1024;

#[global_allocator]
static ALLOC: Cap<System> = Cap::new(System, usize::MAX);
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// `Ok` when the store opened and read whole or there was none, else the error, which must be
/// `Unreadable`.
fn read_all(dir: &Path) -> Result<(), SegError> {
    if let Some(s) = Store::open(dir)? {
        s.check_all()?;
        gitterrace_index::seg::strings::Strings::new(&s.strings).check_all()?;
    }
    Ok(())
}

fn must_not_lie(dir: &Path, case: &str) {
    ALLOC
        .set_limit(ALLOC.allocated() + 2 * MIB)
        .expect("set limit");
    let got = std::panic::catch_unwind(|| read_all(dir));
    ALLOC.set_limit(usize::MAX).expect("lift limit");
    match got {
        Err(payload) => panic!("{case}: the reader panicked: {payload:?}"),
        Ok(Ok(()) | Err(SegError::Unreadable { .. })) => {}
        Ok(Err(e)) => panic!("{case}: {e} is not Unreadable"),
    }
}

/// Recomputes the trailing crc32 of a manifest, so a mutated field passes the crc.
fn reseal_manifest(b: &mut [u8]) {
    let n = b.len() - 4;
    let crc = crc32(&b[..n]);
    b[n..].copy_from_slice(&crc.to_le_bytes());
}

/// Recomputes the footer crc (bytes 0..60 of the last 64) of a container.
fn reseal_footer(b: &mut [u8]) {
    let f = b.len() - 64;
    let crc = crc32(&b[f..f + 60]);
    b[f + 60..].copy_from_slice(&crc.to_le_bytes());
}

/// IEEE crc32, bitwise: the test must not share the reader's implementation.
fn crc32(b: &[u8]) -> u32 {
    let mut c = !0u32;
    for &x in b {
        c ^= u32::from(x);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                (c >> 1) ^ 0xedb8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

#[test]
fn the_crc_here_is_the_standard_one() {
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

#[test]
fn random_bytes_flips_cuts_and_extensions_never_lie() {
    let _one = ONE_AT_A_TIME.lock().expect("lock");
    let d = tempfile::tempdir().expect("tmp");
    let m = common::valid(d.path());
    let names: Vec<String> = m
        .live
        .iter()
        .map(|f| f.name.clone())
        .chain([MANIFEST.to_owned()])
        .collect();
    let good: Vec<Vec<u8>> = names
        .iter()
        .map(|n| fs::read(d.path().join(n)).expect("read"))
        .collect();
    let mut rng = Rng::new(0x5eed);
    for round in 0..ROUNDS {
        let which = rng.below(names.len() as u64) as usize;
        let mut b = good[which].clone();
        let what = rng.below(4);
        match what {
            0 => {
                let n = rng.below(4096) as usize;
                b = (0..n).map(|_| rng.next() as u8).collect();
            }
            1 => {
                for _ in 0..=rng.below(8) {
                    let at = rng.below(b.len() as u64) as usize;
                    b[at] ^= 1 << rng.below(8);
                }
            }
            2 => b.truncate(rng.below(b.len() as u64) as usize),
            _ => b.extend((0..=rng.below(256)).map(|_| rng.next() as u8)),
        }
        fs::write(d.path().join(&names[which]), &b).expect("write");
        must_not_lie(
            d.path(),
            &format!("round {round} kind {what} on {}", names[which]),
        );
        fs::write(d.path().join(&names[which]), &good[which]).expect("restore");
    }
}

/// Every aligned 4 and 8 byte field of the footer and the manifest set to all ones (`u32::MAX`,
/// `u64::MAX`) with the crc recomputed: the bounds, not the crc, must refuse it.
#[test]
fn max_fields_behind_a_valid_crc_never_lie() {
    let _one = ONE_AT_A_TIME.lock().expect("lock");
    let d = tempfile::tempdir().expect("tmp");
    let m = common::valid(d.path());
    for f in &m.live {
        let path = d.path().join(&f.name);
        let good = fs::read(&path).expect("read");
        let foot = good.len() - 64;
        for width in [4, 8] {
            for at in (foot..=foot + 60 - width).step_by(4) {
                let mut b = good.clone();
                b[at..at + width].fill(0xff);
                reseal_footer(&mut b);
                fs::write(&path, &b).expect("write");
                // The manifest pins the footer crc, so also reseal the manifest to reach the
                // footer's own bound checks.
                let mut mm = m.clone();
                for e in &mut mm.live {
                    if e.name == f.name {
                        e.footer_crc = crc32(&b[foot..]);
                    }
                }
                mm.commit(d.path()).expect("manifest");
                must_not_lie(
                    d.path(),
                    &format!("{} footer +{} x{width}", f.name, at - foot),
                );
            }
        }
        fs::write(&path, &good).expect("restore");
        m.clone().commit(d.path()).expect("restore manifest");
    }
    let mp = d.path().join(MANIFEST);
    let good = fs::read(&mp).expect("manifest");
    for width in [1, 4, 8] {
        for at in 0..=good.len() - 4 - width {
            let mut b = good.clone();
            b[at..at + width].fill(0xff);
            reseal_manifest(&mut b);
            fs::write(&mp, &b).expect("write");
            must_not_lie(d.path(), &format!("manifest +{at} x{width}"));
        }
    }
}

/// A commit record that claims 2^24 files, behind a valid block crc, read through the disk
/// source under the 2 MiB cap: it must answer absent and remember `Unreadable`, never
/// allocate for the claimed count. A reader that sizes a buffer by the count aborts here.
#[test]
fn a_record_claiming_millions_of_files_is_unreadable_under_the_cap() {
    use gitterrace_index::feed::{DiskSource, InsightsSource, Rank, disk_testing};
    use gitterrace_index::seg::format::section;
    use gitterrace_index::{CommitRecord, FileStat};

    let _one = ONE_AT_A_TIME.lock().expect("lock");
    let d = tempfile::tempdir().expect("tmp");
    let recs = vec![CommitRecord {
        id: "a".repeat(40),
        author: "A".into(),
        email: "a@x.io".into(),
        time: 1_700_000_000,
        files: vec![FileStat {
            path: "f.rs".into(),
            additions: 1,
            lines: 1,
            ..FileStat::default()
        }],
        ..CommitRecord::default()
    }];
    disk_testing::write_cache(d.path(), &recs, &[]).expect("write cache");
    let store = Store::open(d.path()).expect("open").expect("store");
    let seg = &store.segments[0];
    let e = seg.blocks(section::ROWS)[0];
    let toc_at = seg.toc().iter().position(|t| *t == e).expect("toc entry");
    let name = seg.name().to_owned();
    let mut m = store.manifest.clone();
    drop(store);
    let path = d.path().join(&name);
    let mut b = fs::read(&path).expect("read");
    let at = usize::try_from(e.offset).expect("offset");
    // Record 0: oid 20, ident 4, time 8, offset 4, flags 1, kind 4, scope 4, parent count 2
    // (no parents), then the file count.
    b[at + 47..at + 51].copy_from_slice(&(1u32 << 24).to_le_bytes());
    reseal_block(&mut b, toc_at, at, e.len as usize);
    fs::write(&path, &b).expect("write");
    for f in &mut m.live {
        if f.name == name {
            f.len = b.len() as u64;
            f.footer_crc = crc32(&b[b.len() - 64..]);
        }
    }
    m.commit(d.path()).expect("manifest");
    let store = Store::open(d.path()).expect("open").expect("store");
    let disk = DiskSource::open(store, &mut |pairs| Ok(pairs.to_vec())).expect("disk source");
    ALLOC
        .set_limit(ALLOC.allocated() + 2 * MIB)
        .expect("set limit");
    let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (disk.commit(Rank(0)), disk.rows_for(vec![Rank(0)]).count())
    }));
    ALLOC.set_limit(usize::MAX).expect("lift limit");
    let (commit, rows) = got.expect("the disk source panicked");
    assert!(commit.is_none(), "a hostile record was served");
    assert_eq!(rows, 0);
    assert!(
        disk.unreadable().is_some(),
        "the hostile record was not remembered"
    );
}

/// Recomputes block `toc_index`'s crc in the toc, then the toc crc and the footer crc.
fn reseal_block(b: &mut [u8], toc_index: usize, at: usize, len: usize) {
    let foot = b.len() - 64;
    let field = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().expect("8"));
    let toc = usize::try_from(field(b, foot + 16)).expect("toc offset");
    let count = u32::from_le_bytes(b[foot + 24..foot + 28].try_into().expect("4")) as usize;
    // Toc entry: section u16, zero u16, items u32, offset u64, len u32, crc u32.
    let crc_at = toc + toc_index * 24 + 20;
    let crc = crc32(&b[at..at + len]);
    b[crc_at..crc_at + 4].copy_from_slice(&crc.to_le_bytes());
    let toc_crc = crc32(&b[toc..toc + count * 24]);
    b[foot + 28..foot + 32].copy_from_slice(&toc_crc.to_le_bytes());
    reseal_footer(b);
}

/// The allocator cap is live: a 3 MiB buffer under it fails. Red when the cap is removed.
#[test]
fn the_allocation_cap_is_enforced() {
    let _one = ONE_AT_A_TIME.lock().expect("lock");
    ALLOC
        .set_limit(ALLOC.allocated() + 2 * MIB)
        .expect("set limit");
    let mut v: Vec<u8> = Vec::new();
    let refused = v.try_reserve_exact(3 * MIB).is_err();
    ALLOC.set_limit(usize::MAX).expect("lift limit");
    assert!(refused, "a 3 MiB allocation passed the 2 MiB cap");
}
