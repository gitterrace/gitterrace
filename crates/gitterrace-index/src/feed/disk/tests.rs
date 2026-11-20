//! Unit tests for the disk base.

use std::path::Path;
use std::sync::Arc;

use super::DiskSource;
use crate::feed::{
    CommitRange, IdentId, IdentPairs, InsightsSource, LabelId, MemorySource, PathId, Posting, Rank,
};
use crate::seg::read::Store;
use crate::seg::{SegResult, hex};
use crate::{CodeStat, CommitRecord, FileStat};

static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn oid(i: usize) -> String {
    format!("{:040x}", i * 7919 + 1)
}

fn f(path: &str, add: u32, del: u32) -> FileStat {
    FileStat {
        path: path.into(),
        additions: add,
        deletions: del,
        lines: add * 3,
        rewritten: add.min(del),
        ..FileStat::default()
    }
}

/// Every field the rows carry: merges, renames, code triples, binaries, labels,
/// several parents, two raw identities the mailmap joins, an odd timezone, a far
/// future date (`plausible_time` at read) and an empty commit.
fn fixture() -> Vec<CommitRecord> {
    let mut v = Vec::new();
    for i in 0..40usize {
        let who = ["Ann", "ann", "Bob", "Cy"][i % 4];
        let mut files = vec![f(&format!("src/m{}.rs", i % 7), i as u32, (i % 3) as u32)];
        if i % 5 == 0 {
            files.push(FileStat {
                old: Some(format!("old/{}.rs", i % 4).into()),
                code: Some(CodeStat { additions: 1, deletions: 0, rewritten: 0 }),
                ..f(&format!("new/{i}.rs"), 2, 2)
            });
        }
        if i % 9 == 0 {
            files.push(FileStat { binary: true, ..f("img.png", 0, 0) });
        }
        if i == 13 {
            files.clear();
        }
        v.push(CommitRecord {
            id: oid(i),
            author: who.into(),
            email: if who == "ann" { "ANN@x.io".into() } else { format!("{who}@x.io") },
            time: if i == 30 { 4_000_000_000 } else { 1_700_000_000 + (i as i64) * 50_000 },
            offset: if i % 2 == 0 { 3600 } else { -19_800 },
            merge: i % 11 == 10,
            parents: (0..(i.min(1) + usize::from(i % 11 == 10)))
                .map(|k| oid(i.saturating_sub(k + 1)))
                .collect(),
            kind: (i % 3 == 0).then(|| ["feat", "fix"][i % 2].into()),
            scope: (i % 6 == 0).then(|| "web".into()),
            files,
        });
    }
    v
}

fn mailmap(recs: &[CommitRecord]) -> IdentPairs {
    recs.iter()
        .map(|r| {
            let raw = (r.author.clone(), r.email.clone());
            let shown = if r.email.eq_ignore_ascii_case("ann@x.io") {
                ("Ann".to_owned(), "ann@x.io".to_owned())
            } else {
                raw.clone()
            };
            (raw, shown)
        })
        .collect()
}

use super::testing::write_cache;

fn open(dir: &Path, map: &IdentPairs) -> SegResult<DiskSource> {
    let store = Store::open(dir)?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
    DiskSource::open(store, &mut |pairs| {
        Ok(pairs.iter().map(|p| map.get(p).cloned().unwrap_or_else(|| p.clone())).collect())
    })
}

/// Every trait method, both sides, over every id and rank plus out-of-range ones.
#[expect(clippy::many_single_char_names, reason = "d and m are the two sides throughout")]
fn assert_same(d: &dyn InsightsSource, m: &dyn InsightsSource, case: &str) {
    assert_eq!(d.len(), m.len(), "{case}: len");
    let n = m.len() as u32;
    for r in 0..n + 2 {
        let r = Rank(r);
        assert_eq!(d.commit(r), m.commit(r), "{case}: commit {r:?}");
        assert_eq!(d.commit_id(r), m.commit_id(r), "{case}: id {r:?}");
        assert_eq!(d.parent_ids(r), m.parent_ids(r), "{case}: parents {r:?}");
        if let Some(id) = m.commit_id(r) {
            assert_eq!(d.find_commit(&id), m.find_commit(&id), "{case}: find {id}");
        }
    }
    assert_eq!(d.find_commit(&oid(99_999)), Ok(None), "{case}: absent commit");
    for range in [
        CommitRange::All,
        CommitRange::Ranks(3..17),
        CommitRange::Ranks(n.saturating_sub(2)..n + 5),
        CommitRange::Time(1_700_100_000..1_700_900_000),
    ] {
        let a: Vec<_> = d.commits_in(range.clone()).collect();
        let b: Vec<_> = m.commits_in(range.clone()).collect();
        assert_eq!(a, b, "{case}: commits_in {range:?}");
    }
    let ranks: Vec<Rank> = (0..=n).map(Rank).collect();
    let a: Vec<_> = d.rows_for(ranks.clone()).collect();
    let b: Vec<_> = m.rows_for(ranks).collect();
    assert_eq!(a, b, "{case}: rows_for");
    for days in [i64::MIN..i64::MAX, 19_680..19_700] {
        let a: Vec<_> = d.agg_author_day(days.clone()).collect();
        let b: Vec<_> = m.agg_author_day(days).collect();
        assert_eq!(a, b, "{case}: author day");
    }
    for p in ["", "src", "new/", "zz"] {
        let a: Vec<_> = d.agg_path_author(p).collect();
        let b: Vec<_> = m.agg_path_author(p).collect();
        assert_eq!(a, b, "{case}: path author {p}");
    }
    assert_eq!(d.paths(), m.paths(), "{case}: paths");
    for p in 0..m.paths() as u32 {
        let s = m.path(PathId(p));
        assert_eq!(d.path(PathId(p)), s, "{case}: path {p}");
        assert_eq!(s.and_then(|s| d.path_id(&s)), Some(PathId(p)), "{case}: find {p}");
    }
    // Postings are a full scan here (served from segments in production), so probe the
    // first 64 paths and the last, not 60k of the wide case.
    let np = m.paths() as u32;
    for p in (0..np.min(64)).chain([np.saturating_sub(1), np]) {
        let id = PathId(p);
        assert_eq!(d.path(id), m.path(id), "{case}: path {p}");
        if let Some(s) = m.path(id) {
            assert_eq!(d.find_path(&s), m.find_path(&s), "{case}: find path {s}");
        }
        let a: Vec<_> = d.postings(Posting::Path(id)).collect();
        let b: Vec<_> = m.postings(Posting::Path(id)).collect();
        assert_eq!(a, b, "{case}: path postings {p}");
    }
    assert_eq!(d.find_path("nope"), m.find_path("nope"), "{case}: absent path");
    assert_eq!(d.idents(), m.idents(), "{case}: idents");
    for i in 0..=m.idents() as u32 {
        assert_eq!(d.ident(IdentId(i)), m.ident(IdentId(i)), "{case}: ident {i}");
        let a: Vec<_> = d.postings(Posting::Author(IdentId(i))).collect();
        let b: Vec<_> = m.postings(Posting::Author(IdentId(i))).collect();
        assert_eq!(a, b, "{case}: author postings {i}");
    }
    for l in 0..4 {
        let id = LabelId(l);
        assert_eq!(d.label(id), m.label(id), "{case}: label {l}");
        if let Some(s) = m.label(id) {
            assert_eq!(d.find_label(&s), m.find_label(&s), "{case}: find label {s}");
        }
    }
    assert_eq!(d.find_label("chore"), m.find_label("chore"), "{case}: absent label");
    let a: Vec<_> = d.rename_edges().collect();
    let b: Vec<_> = m.rename_edges().collect();
    assert_eq!(a, b, "{case}: rename edges");
}

/// `DiskSource` equals `MemorySource` (raw identities plus the mailmap, the one rule) on
/// every trait method, over the option matrix the source can see: mailmap as is and
/// edited, one segment and an unmerged append set, a record over the block size, and
/// a seeded set of random histories. Options (bots, filters, whitespace, period) apply
/// above the source to the raw facts compared here.
#[test]
fn disk_equals_memory_on_every_method() -> SegResult<()> {
    let _one = ONE_AT_A_TIME.lock();
    let mut cases: Vec<(String, Vec<CommitRecord>)> = vec![("fixture".into(), fixture())];
    // A commit of 60k files spans two 1 MiB row blocks (the join path).
    let mut big = fixture();
    big[20].files = (0..60_000).map(|k| f(&format!("wide/{k}"), 1, 0)).collect();
    cases.push(("big record".into(), big));
    let big_only = |name: &str, edited: bool, cuts: &[usize]| {
        name != "big record" || (!edited && cuts.len() == 1)
    };
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for k in 0..6 {
        let mut v = fixture();
        for r in &mut v {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            r.offset = (seed % 50_000) as i32 - 25_000;
            r.files.truncate((seed % 4) as usize);
            if seed.is_multiple_of(5) {
                r.kind = None;
            }
        }
        cases.push((format!("random {k}"), v));
    }
    for (name, recs) in cases {
        for edited in [false, true] {
            let map = if edited { mailmap(&recs) } else { IdentPairs::new() };
            // Index::records hands the oracle clamped times, the disk clamps at read.
            let latest = 1_800_000_000;
            let mut shown = recs.clone();
            for r in &mut shown {
                r.time = crate::plausible_time(r.time, latest);
            }
            let mem = MemorySource::shared_raw(Arc::new(shown), map.clone());
            for cuts in [vec![], vec![17], vec![5, 30]] {
                if !big_only(&name, edited, &cuts) {
                    continue;
                }
                let dir = tempfile::tempdir()?;
                write_cache(dir.path(), &recs, &cuts)?;
                let mut disk = open(dir.path(), &map)?;
                disk.latest = latest;
                let case = format!("{name}, mailmap edited {edited}, cuts {cuts:?}");
                assert_same(&disk, &mem, &case);
                assert_eq!(disk.unreadable(), None, "{case}");
                let back: Vec<CommitRecord> = disk.records().collect::<SegResult<_>>()?;
                for (a, b) in back.iter().zip(&recs) {
                    assert_eq!(format!("{a:?}"), format!("{b:?}"), "{case}: record");
                }
                let probe = crate::store::oid_of(&recs[7].id).unwrap_or_default();
                let got = disk.record_by_oid(&probe)?.map(|r| r.id);
                assert_eq!(got.as_deref(), Some(recs[7].id.as_str()), "{case}");
            }
        }
    }
    // The rule itself: joined raw identities keep two ids with equal strings.
    let recs = fixture();
    let mem = MemorySource::shared_raw(Arc::new(recs.clone()), mailmap(&recs));
    assert_eq!(mem.ident(IdentId(0)), mem.ident(IdentId(1)));
    Ok(())
}

fn flip_in_rows(dir: &Path) -> SegResult<()> {
    // Byte 24 of the first record is the low byte of its time.
    flip_in(dir, crate::seg::format::section::ROWS, 24)
}

/// Flips byte `at` of the first block of `sec` in the first segment.
fn flip_in(dir: &Path, sec: u16, at: usize) -> SegResult<()> {
    let store = Store::open(dir)?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
    let seg = &store.segments[0];
    let e = seg.blocks(sec)[0];
    let path = dir.join(seg.name());
    let mut bytes = std::fs::read(&path)?;
    bytes[e.offset as usize + at] ^= 1;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// A flipped byte in the commit columns fails every commit read, never serves a wrong
/// time: a commit pass reads only the columns.
#[test]
fn corrupt_commit_columns_are_unreadable_never_wrong() -> SegResult<()> {
    let _one = ONE_AT_A_TIME.lock();
    let dir = tempfile::tempdir()?;
    write_cache(dir.path(), &fixture(), &[])?;
    // Byte 4 of the first item is the low byte of its time.
    flip_in(dir.path(), crate::seg::format::section::COMMITS, 4)?;
    let disk = open(dir.path(), &IdentPairs::new())?;
    assert!(disk.commits_in(CommitRange::All).next().is_none());
    assert!(disk.unreadable().is_some_and(|e| e.contains("crc")));
    assert_eq!(disk.commit(Rank(0)), None);
    Ok(())
}

/// A flipped byte in a row block reads as absent plus `unreadable()` (the index then
/// rebuilds), never as a commit with a wrong value.
#[test]
fn corrupt_row_block_is_unreadable_never_wrong() -> SegResult<()> {
    let _one = ONE_AT_A_TIME.lock();
    let recs = fixture();
    let dir = tempfile::tempdir()?;
    write_cache(dir.path(), &recs, &[])?;
    flip_in_rows(dir.path())?;
    let disk = open(dir.path(), &IdentPairs::new())?;
    assert_eq!(disk.unreadable(), None);
    assert_eq!(disk.commit(Rank(0)), None);
    assert!(disk.unreadable().is_some_and(|e| e.contains("crc")));
    assert_eq!(disk.rows_for(vec![Rank(0)]).count(), 0);
    let all: Vec<Rank> = (0..disk.len()).map(|r| Rank(r as u32)).collect();
    assert_eq!(disk.rows_for(all).count(), 0, "a scan with rows stops at the bad block");
    Ok(())
}

/// A record claiming 2^24 files with a valid crc fails at the end of its bytes (the
/// allocation cap is in `tests/disk_memory.rs`), and a row index pointing past the rows or
/// backwards is `Unreadable`.
#[test]
fn hostile_counts_and_offsets_are_unreadable() -> SegResult<()> {
    let _one = ONE_AT_A_TIME.lock();
    let recs = fixture();
    for what in ["nfiles", "offset"] {
        let dir = tempfile::tempdir()?;
        write_cache(dir.path(), &recs, &[])?;
        let store =
            Store::open(dir.path())?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
        let seg = &store.segments[0];
        let sec = if what == "nfiles" {
            crate::seg::format::section::ROWS
        } else {
            crate::seg::format::section::ROW_INDEX
        };
        let e = seg.blocks(sec)[0];
        let name = seg.name().to_owned();
        let toc_at = seg.toc().iter().position(|t| *t == e).unwrap_or(0);
        drop(store);
        let path = dir.path().join(&name);
        let mut b = std::fs::read(&path)?;
        let at = e.offset as usize;
        if what == "nfiles" {
            // Record 0: oid 20, ident 4, time 8, offset 4, flags 1, kind 4, scope 4,
            // parent count 2 (0 parents), then the file count.
            b[at + 47..at + 51].copy_from_slice(&(1u32 << 24).to_le_bytes());
        } else {
            b[at + 8..at + 16].copy_from_slice(&u64::MAX.to_le_bytes());
        }
        reseal(&mut b, toc_at, at, e.len as usize);
        std::fs::write(&path, &b)?;
        let store =
            Store::open(dir.path())?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
        let mut entry = store.manifest.clone();
        drop(store);
        for f in &mut entry.live {
            if f.name == name {
                f.len = b.len() as u64;
                f.footer_crc = crc32fast::hash(&b[b.len() - 64..]);
            }
        }
        entry.commit(dir.path())?;
        let disk = open(dir.path(), &IdentPairs::new())?;
        let got = disk.commit(Rank(0));
        let rows = disk.rows_for(vec![Rank(0)]).count();
        assert_eq!((got, rows), (None, 0), "{what}");
        assert!(disk.unreadable().is_some(), "{what}");
    }
    Ok(())
}

/// Section 1 fields set behind a valid crc: a flag outside the defined ones, a reserved
/// byte, or a value that disagrees with `ROWS`. Every read that sees both sections (point
/// lookups, rows) reads it as absent plus `unreadable()`, and a malformed item also stops a
/// commit pass.
#[test]
fn hostile_commit_columns_are_unreadable() -> SegResult<()> {
    let _one = ONE_AT_A_TIME.lock();
    let recs = fixture();
    // (byte offset in item 0, value): flags, reserved u8, time low byte, sums low byte.
    for (at_item, val) in [(16usize, 7u8), (17, 1), (4, 0x5a), (32, 0xff)] {
        let dir = tempfile::tempdir()?;
        write_cache(dir.path(), &recs, &[])?;
        let store =
            Store::open(dir.path())?.ok_or_else(|| crate::seg::SegError::BadName("none".into()))?;
        let seg = &store.segments[0];
        let e = seg.blocks(crate::seg::format::section::COMMITS)[0];
        let name = seg.name().to_owned();
        let toc_at = seg.toc().iter().position(|t| *t == e).unwrap_or(0);
        let mut entry = store.manifest.clone();
        drop(store);
        let path = dir.path().join(&name);
        let mut b = std::fs::read(&path)?;
        b[e.offset as usize + at_item] ^= val;
        reseal(&mut b, toc_at, e.offset as usize, e.len as usize);
        std::fs::write(&path, &b)?;
        for f in &mut entry.live {
            if f.name == name {
                f.footer_crc = crc32fast::hash(&b[b.len() - 64..]);
            }
        }
        entry.commit(dir.path())?;
        let disk = open(dir.path(), &IdentPairs::new())?;
        assert_eq!(disk.commit(Rank(0)), None, "byte {at_item}");
        assert!(disk.unreadable().is_some(), "byte {at_item}");
        let disk = open(dir.path(), &IdentPairs::new())?;
        let all: Vec<Rank> = (0..disk.len()).map(|r| Rank(r as u32)).collect();
        assert!(
            disk.rows_for(all).all(|(r, _)| r != Rank(0)),
            "byte {at_item}: the bad commit is never served"
        );
        assert!(disk.unreadable().is_some(), "byte {at_item}");
        // A commit pass reads section 1 alone: a malformed item stops it. A well-formed
        // value behind a valid crc is, like a crafted `ROWS` value, not detectable there.
        if (16..20).contains(&at_item) {
            let disk = open(dir.path(), &IdentPairs::new())?;
            assert!(disk.commits_in(CommitRange::All).next().is_none(), "byte {at_item}");
            assert!(disk.unreadable().is_some(), "byte {at_item}");
        }
    }
    Ok(())
}

/// Recomputes a block crc in the toc, the toc crc and the footer crc.
fn reseal(b: &mut [u8], toc_index: usize, at: usize, len: usize) {
    let n = b.len();
    let foot = n - 64;
    let toc_off = u64::from_le_bytes(b[foot + 16..foot + 24].try_into().unwrap_or_default());
    let toc_count = u32::from_le_bytes(b[foot + 24..foot + 28].try_into().unwrap_or_default());
    let toc = toc_off as usize;
    let crc = crc32fast::hash(&b[at..at + len]);
    let entry = toc + toc_index * 24;
    b[entry + 20..entry + 24].copy_from_slice(&crc.to_le_bytes());
    let toc_crc = crc32fast::hash(&b[toc..toc + toc_count as usize * 24]);
    b[foot + 28..foot + 32].copy_from_slice(&toc_crc.to_le_bytes());
    let fcrc = crc32fast::hash(&b[foot..foot + 60]);
    b[foot + 60..].copy_from_slice(&fcrc.to_le_bytes());
}

/// Past one `OID_INDEX` block (43690 entries), every commit is found in whichever block it
/// routes to, and an oid between two blocks' boundaries or past the last is absent, not an
/// error: the boundary checks pass on a sound set.
#[test]
fn lookups_across_oid_blocks_find_every_commit() -> SegResult<()> {
    let _one = ONE_AT_A_TIME.lock();
    let recs: Vec<CommitRecord> = (0..45_000usize)
        .map(|i| CommitRecord {
            id: oid(i),
            author: "A".into(),
            email: "a@x.io".into(),
            time: 1_700_000_000 + i as i64,
            ..CommitRecord::default()
        })
        .collect();
    let dir = tempfile::tempdir()?;
    write_cache(dir.path(), &recs, &[])?;
    let disk = open(dir.path(), &IdentPairs::new())?;
    assert_eq!(disk.segs[0].oid_first.len(), 2, "two oid blocks");
    for (i, r) in recs.iter().enumerate().step_by(97).chain(recs.iter().enumerate().skip(44_990)) {
        assert!(disk.find_commit(&r.id).ok().flatten().is_some(), "commit {i}");
    }
    let mut ids: Vec<&str> = recs.iter().map(|r| r.id.as_str()).collect();
    ids.sort_unstable();
    let boundary = hex(&disk.segs[0].oid_first[1]);
    assert!(disk.find_commit(&boundary).is_ok_and(|r| r.is_some()));
    for absent in [format!("{:040x}", 0), "f".repeat(40), oid(99_999_999)] {
        assert!(ids.binary_search(&absent.as_str()).is_err());
        assert_eq!(disk.find_commit(&absent), Ok(None), "{absent}");
    }
    assert_eq!(disk.unreadable(), None);
    Ok(())
}
