//! Unit tests for the record store.

use std::fs;
use std::io;

use crate::seg::format::section;

use super::{FORMAT_VERSION, LOCK_FILE, VERSION_FILE, ensure_dir};

fn names(dir: &std::path::Path) -> io::Result<Vec<String>> {
    let mut out: Vec<String> = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<_>>()?;
    out.sort();
    Ok(out)
}

/// An older format deletes everything it held, not only the files this build knows.
#[test]
fn an_unreadable_cache_is_deleted_whole() -> io::Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("gitterrace");
    fs::create_dir(&dir)?;
    fs::write(dir.join(VERSION_FILE), "5\n")?;
    // The record files of format 5 and before go with everything else.
    fs::write(dir.join("commits.jsonl"), "{}\n")?;
    fs::write(dir.join("paths.jsonl"), "\"a\"\n")?;
    fs::write(dir.join("seg-1-0.gts"), "old segment")?;
    assert_eq!(ensure_dir(&dir)?, None);
    assert_eq!(names(&dir)?, [VERSION_FILE, LOCK_FILE]);
    assert_eq!(
        fs::read_to_string(dir.join(VERSION_FILE))?.trim(),
        FORMAT_VERSION
    );
    Ok(())
}

/// A newer build's cache stays byte for byte, stray files included.
#[test]
fn a_newer_cache_stays_untouched() -> io::Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("gitterrace");
    fs::create_dir(&dir)?;
    fs::write(dir.join(VERSION_FILE), "999\n")?;
    fs::write(dir.join("seg-9-0.gts"), "future")?;
    assert_eq!(ensure_dir(&dir)?.as_deref(), Some("999"));
    assert_eq!(names(&dir)?, ["VERSION", "seg-9-0.gts"]);
    Ok(())
}

/// A built segment set survives reopening whole. One flipped byte in a live file wipes the
/// whole cache, so the set is never partly served.
#[test]
fn a_built_segment_set_is_kept_and_a_corrupt_one_wiped() -> io::Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("gitterrace");
    assert_eq!(ensure_dir(&dir)?, None);
    let mut b = super::SegmentBuild::new(&dir, None, &[]).map_err(io::Error::other)?;
    for (i, path) in ["a.rs", "b.rs", "a.rs"].into_iter().enumerate() {
        let r = crate::CommitRecord {
            id: format!("{i:040x}"),
            author: "Ann".into(),
            email: "ann@x".into(),
            time: 1_700_000_000 + i64::try_from(i).unwrap_or(0) * 86_400,
            offset: 0,
            merge: false,
            parents: Vec::new(),
            kind: Some("feat".into()),
            scope: None,
            files: vec![crate::FileStat {
                path: path.into(),
                additions: 1,
                deletions: 0,
                lines: 1,
                rewritten: 0,
                code: None,
                old: None,
                binary: false,
            }],
        };
        b.push(&r).map_err(io::Error::other)?;
    }
    let m = b.finish(None, Vec::new()).map_err(io::Error::other)?;
    let live: Vec<&str> = m.live.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(live, ["seg-1-0.gts", "str-1-0.gts", "ord-1.gts"]);
    assert_eq!(m.commits, 3);
    assert_eq!(ensure_dir(&dir)?, None);
    assert_eq!(
        names(&dir)?,
        [
            "MANIFEST",
            VERSION_FILE,
            LOCK_FILE,
            "ord-1.gts",
            "seg-1-0.gts",
            "str-1-0.gts"
        ]
    );
    let seg = dir.join("seg-1-0.gts");
    let mut bytes = fs::read(&seg)?;
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&seg, bytes)?;
    assert_eq!(ensure_dir(&dir)?, None);
    assert_eq!(names(&dir)?, [VERSION_FILE, LOCK_FILE]);
    Ok(())
}

/// A deepened (or newly shallow) clone changes topo ranks: the shallow fingerprint in the
/// manifest then differs and the cache is wiped like an unreadable one.
#[test]
fn a_changed_shallow_set_rebuilds() -> io::Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("gitterrace");
    assert_eq!(ensure_dir(&dir)?, None);
    let boundary = vec![format!("{:040x}", 7)];
    let b = super::SegmentBuild::new(&dir, None, &boundary).map_err(io::Error::other)?;
    let m = b.finish(None, Vec::new()).map_err(io::Error::other)?;
    assert_ne!(m.shallow, 0);
    assert!(!super::check_shallow(&dir, &boundary)?);
    assert!(names(&dir)?.contains(&"MANIFEST".to_owned()));
    assert!(super::check_shallow(&dir, &[])?);
    assert_eq!(names(&dir)?, [VERSION_FILE, LOCK_FILE]);
    Ok(())
}

fn rec(i: usize, path: &str) -> crate::CommitRecord {
    crate::CommitRecord {
        id: format!("{i:040x}"),
        author: if i.is_multiple_of(2) { "Ann" } else { "Bob" }.into(),
        email: "x@x".into(),
        time: 1_700_000_000 + i64::try_from(i).unwrap_or(0) * 86_400,
        offset: 0,
        merge: false,
        parents: Vec::new(),
        kind: Some("fix".into()),
        scope: Some(format!("s{}", i % 3)),
        files: vec![crate::FileStat {
            path: path.into(),
            additions: 2,
            deletions: 1,
            lines: 3,
            rewritten: 0,
            code: None,
            old: None,
            binary: false,
        }],
    }
}

fn build(dir: &std::path::Path, recs: &[crate::CommitRecord]) -> io::Result<Vec<Vec<u8>>> {
    let prev = crate::seg::manifest::Manifest::load(dir).map_err(io::Error::other)?;
    let mut b = super::SegmentBuild::new(dir, prev.as_ref(), &[]).map_err(io::Error::other)?;
    for r in recs {
        b.push(r).map_err(io::Error::other)?;
    }
    let m = b.finish(prev, Vec::new()).map_err(io::Error::other)?;
    m.live.iter().map(|f| fs::read(dir.join(&f.name))).collect()
}

/// Twenty small updates, each appended as a segment, merged in the background whenever the
/// set holds more than eight. Right after every merge the bytes equal a one-step build of
/// the same commits. Some updates bring commits dated before ones already stored (a merged
/// branch), and some are merge commits, so the merge must renumber into canonical order.
#[test]
fn twenty_updates_with_background_merges_equal_one_step() -> io::Result<()> {
    let mut all: Vec<crate::CommitRecord> = Vec::new();
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("gitterrace");
    assert_eq!(ensure_dir(&dir)?, None);
    let mut merges = 0;
    for u in 0..20usize {
        // Two commits per update. Every fourth update's second commit is dated before
        // everything stored so far, every fifth update's first commit is a merge.
        let mut batch = vec![rec(100 + 2 * u, &format!("p{}", u % 5))];
        let mut old = rec(2 * u + 1, &format!("q{u}"));
        if u % 4 == 3 {
            old.time = 1_600_000_000 + i64::try_from(u).unwrap_or(0);
        }
        batch.push(old);
        if u % 5 == 4 {
            let extra = crate::FileStat {
                path: format!("m{u}").into(),
                ..batch[0].files[0].clone()
            };
            batch[0].merge = true;
            batch[0].files.push(extra);
        }
        all.extend(batch.iter().cloned());
        match crate::seg::manifest::Manifest::load(&dir).map_err(io::Error::other)? {
            None => {
                let mut b = super::SegmentBuild::new(&dir, None, &[]).map_err(io::Error::other)?;
                for r in &batch {
                    b.push(r).map_err(io::Error::other)?;
                }
                b.finish(None, Vec::new()).map_err(io::Error::other)?;
            }
            Some(prev) => {
                super::append_segment(&dir, prev, &[], &batch).map_err(io::Error::other)?;
            }
        }
        // No parents in this fixture, so the canonical order is (time, oid).
        let mut canonical = all.clone();
        canonical.sort_by(|a, b| (a.time, &a.id).cmp(&(b.time, &b.id)));
        let ids: Vec<String> = canonical.iter().map(|r| r.id.clone()).collect();
        let by_id: std::collections::HashMap<String, crate::CommitRecord> =
            all.iter().map(|r| (r.id.clone(), r.clone())).collect();
        let mut fetch = |id: &str| {
            by_id
                .get(id)
                .cloned()
                .ok_or_else(|| io::Error::other("missing record"))
        };
        if super::merge_if_needed(&dir, &ids, &[], Vec::new(), &mut fetch)? {
            merges += 1;
            let one = tempfile::tempdir()?;
            let want = build(one.path(), &canonical)?;
            let m = crate::seg::manifest::Manifest::load(&dir)
                .map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("no manifest"))?;
            let got: Vec<Vec<u8>> = m
                .live
                .iter()
                .map(|f| fs::read(dir.join(&f.name)))
                .collect::<io::Result<_>>()?;
            assert_eq!(got, want, "after update {u}");
            assert_eq!(m.live.len(), 3, "one segment, one delta, one order file");
        }
        // The set itself must stay readable whole between merges.
        crate::seg::read::Store::open(&dir).map_err(io::Error::other)?;
        let segs = names(&dir)?
            .iter()
            .filter(|n| n.starts_with("seg-"))
            .count();
        assert!(
            segs <= super::MAX_SEGMENTS,
            "{segs} segments after update {u}"
        );
    }
    assert_eq!(merges, 2);
    Ok(())
}

/// Two-step (HEAD~k, then HEAD) followed by the merge equals a one-step build byte for
/// byte: the merge renumbers by rebuilding from the records in canonical order, so ids and
/// ordinals cannot depend on the build history. Retired files of the steps are gone.
#[test]
fn two_steps_then_merge_equal_one_step() -> io::Result<()> {
    let recs: Vec<_> = ["a", "b", "c", "a", "d", "b", "e"]
        .iter()
        .enumerate()
        .map(|(i, p)| rec(i, p))
        .collect();
    let one = tempfile::tempdir()?;
    let want = build(one.path(), &recs)?;
    let two = tempfile::tempdir()?;
    build(two.path(), &recs[..3])?;
    build(two.path(), &recs[3..])?;
    let got = build(two.path(), &recs)?;
    assert_eq!(got, want);
    // The merge rewrites the commit rows too, one row per commit, not only the tables.
    let store = crate::seg::read::Store::open(two.path())
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other("no set"))?;
    let seg = &store.segments[0];
    assert!(!seg.blocks(section::ROWS).is_empty(), "no row section");
    for sec in [section::ROW_INDEX, section::OID_INDEX] {
        let n: u32 = seg.blocks(sec).iter().map(|e| e.items).sum();
        assert_eq!(n as usize, recs.len(), "section {sec}");
    }
    assert!(
        !names(two.path())?
            .iter()
            .any(|n| n.starts_with("seg-1") || n.starts_with("seg-2"))
    );
    Ok(())
}

/// A current cache keeps its live files, loses a retired or stray one, and keeps a temp file
/// that may be another process's write in progress.
#[test]
fn the_sweep_removes_only_files_not_live() -> io::Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("gitterrace");
    fs::create_dir(&dir)?;
    fs::write(dir.join(VERSION_FILE), format!("{FORMAT_VERSION}\n"))?;
    fs::write(dir.join(LOCK_FILE), "")?;
    fs::write(dir.join("retired-seg.gts"), "x")?;
    fs::write(dir.join(".lines.json.1.2.tmp"), "x")?;
    assert_eq!(ensure_dir(&dir)?, None);
    assert_eq!(
        names(&dir)?,
        [".lines.json.1.2.tmp", VERSION_FILE, LOCK_FILE]
    );
    Ok(())
}
