//! Resident memory of the disk base source: it must not grow with history.
//! A counting global allocator (the `cap` crate, own test binary) measures live heap bytes.

use std::alloc::System;
use std::path::Path;

use cap::Cap;
use gitterrace_index::feed::{CommitRange, DiskSource, InsightsSource, PathId, Rank};
use gitterrace_index::seg::read::Store;
use gitterrace_index::seg::{SegError, SegResult};
use gitterrace_index::{CommitRecord, FileStat};

#[global_allocator]
static ALLOC: Cap<System> = Cap::new(System, usize::MAX);

const MIB: usize = 1 << 20;

fn oid(i: usize) -> String {
    format!("{:040x}", i * 7919 + 1)
}

/// `n` commits over a fixed 500 paths and 20 people, so only the history grows.
fn synthetic(n: usize) -> Vec<CommitRecord> {
    (0..n)
        .map(|i| CommitRecord {
            id: oid(i),
            author: format!("p{}", i % 20),
            email: format!("p{}@x.io", i % 20),
            time: 1_600_000_000 + i as i64 * 600,
            parents: if i == 0 { vec![] } else { vec![oid(i - 1)] },
            kind: Some("feat".into()),
            files: (0..3)
                .map(|k| FileStat {
                    path: format!("d{k}/f{}", (i * 7 + k) % 500).into(),
                    additions: 3,
                    deletions: 1,
                    lines: 9,
                    ..FileStat::default()
                })
                .collect(),
            ..CommitRecord::default()
        })
        .collect()
}

fn open(dir: &Path) -> SegResult<DiskSource> {
    let store = Store::open(dir)?.ok_or_else(|| SegError::BadName("no manifest".into()))?;
    DiskSource::open(store, &mut |p| Ok(p.to_vec()))
}

/// Live heap kept after a sweep of every read a page makes, and the peak during it.
fn footprint(n: usize) -> SegResult<(usize, usize)> {
    let dir = tempfile::tempdir()?;
    let recs = synthetic(n);
    gitterrace_index::feed::disk_testing::write_cache(dir.path(), &recs, &[])?;
    drop(recs);
    let before = ALLOC.allocated();
    let mut peak = before;
    let mut probe = || peak = peak.max(ALLOC.allocated());
    let disk = open(dir.path())?;
    let len = u32::try_from(disk.len()).unwrap_or(u32::MAX);
    let mut sink = 0u64;
    for c in disk.commits_in(CommitRange::All) {
        sink += c.sums.add;
    }
    probe();
    for (_, rows) in disk.rows_for((0..len).map(Rank).collect()) {
        sink += rows.len() as u64;
    }
    probe();
    sink += disk.agg_author_day(i64::MIN..i64::MAX).count() as u64;
    probe();
    sink += disk.agg_path_author("").count() as u64;
    probe();
    for p in 0..u32::try_from(disk.paths()).unwrap_or(u32::MAX) {
        if let Some(s) = disk.path(PathId(p)) {
            sink += u64::from(disk.path_id(&s).is_some());
        }
    }
    probe();
    for r in (0..len).step_by(97) {
        if let Some(id) = disk.commit_id(Rank(r)) {
            sink += u64::from(disk.commit_of(&id).is_some());
        }
    }
    probe();
    assert!(sink > 0);
    assert_eq!(disk.unreadable(), None);
    let kept = ALLOC.allocated().saturating_sub(before);
    drop(disk);
    Ok((kept, peak.saturating_sub(before)))
}

/// History grows 10x, resident memory does not pass a ceiling that is independent of it:
/// the 12 MiB block cache plus 2 MiB of per-file tables, well inside the 32 MB budget plus
/// 16 MB margin. Seen red with the cache unbounded (`block_cache_default` = `usize::MAX`): the 400k
/// history then keeps every row block (about 40 MB).
#[test]
fn resident_memory_is_flat_with_history() -> SegResult<()> {
    let (k1, p1) = footprint(40_000)?;
    let (k10, p10) = footprint(400_000)?;
    let ceiling = 14 * MIB;
    for (n, k, p) in [(40_000, k1, p1), (400_000, k10, p10)] {
        assert!(
            k <= ceiling && p <= ceiling,
            "{n} commits: kept {k}, peak {p}"
        );
    }
    Ok(())
}
