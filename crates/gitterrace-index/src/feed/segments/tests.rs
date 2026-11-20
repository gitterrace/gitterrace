//! Unit tests for the segment builder.

use std::path::Path;

use super::{SegmentSource, write_tables};
use crate::CommitRecord;
use crate::feed::{InsightsSource, MemorySource, Posting};
use crate::insights::fixtures::{file, rec};
use crate::seg::SegResult;
use crate::seg::agg::{Aggregates, Strings};
use crate::seg::format::{Kind, StringBase};
use crate::seg::postings::Postings;
use crate::seg::read::Container;
use crate::seg::write::ContainerWriter;

fn renamed(new: &str, old: &str) -> crate::FileStat {
    let mut f = file(new, 1, 1, 5);
    f.old = Some(old.into());
    f
}

fn history() -> Vec<CommitRecord> {
    let mut m = rec("m", "Bob", 6, vec![file("a", 1, 0, 1)]);
    m.merge = true;
    vec![
        rec("1", "Ann", 0, vec![file("a", 1, 0, 1), file("b", 1, 0, 1)]),
        rec("2", "Ann", 1, vec![renamed("c", "a")]),
        rec("3", "Bob", 2, vec![file("a", 2, 0, 2), file("c", 1, 0, 2)]),
        rec("4", "Bob", 3, vec![renamed("a", "b"), renamed("b", "a")]),
        rec("5", "Ann", 40, vec![renamed("d", "c"), file("a", 1, 1, 2)]),
        m,
        rec("6", "Cy", 41, vec![renamed("e", "d")]),
    ]
}

/// One segment per slice of `recs`, `ranks` giving each commit's canonical position.
fn write(
    dir: &Path,
    s: &mut Strings,
    n: usize,
    recs: &[&CommitRecord],
    ranks: &[u32],
) -> SegResult<Container> {
    let (mut a, mut p) = (Aggregates::default(), Postings::default());
    for r in recs {
        let (who, paths) = s.ids(r);
        let olds = crate::seg::canon::olds(s, r);
        a.push(r, who, &paths, &olds);
        p.push(s, r, who, &paths);
    }
    let name = format!("seg-1-{n}.gts");
    let mut w = ContainerWriter::new(dir, &name, Kind::Segment, StringBase::default())?;
    let n_paths = s.paths.list.len();
    write_tables(&a, &p, ranks, n_paths, &mut |sec, items, b| w.block(sec, items, b))?;
    w.finish()?;
    Container::open(dir, &name, Kind::Segment)
}

/// Every aggregate, posting and edge from one segment and from an unmerged two-segment
/// set (a branch written later, dated earlier) equals the memory adapter.
#[test]
fn segments_answer_like_the_memory_adapter() -> SegResult<()> {
    let recs = history();
    let mem = MemorySource::new(&recs);
    let all: Vec<&CommitRecord> = recs.iter().collect();
    let dir = tempfile::tempdir()?;
    let mut s = Strings::default();
    let one = vec![write(dir.path(), &mut s, 0, &all, &[0, 1, 2, 3, 4, 5, 6])?];
    // Two-step: commits 1 2 4 5 m 6 first, then the branch commit 3. Path ids still
    // follow the memory adapter's first use, as the cache's string deltas would.
    let dir2 = tempfile::tempdir()?;
    let mut s2 = Strings::default();
    for r in &recs {
        s2.ids(r);
    }
    let first: Vec<&CommitRecord> = [0, 1, 3, 4, 5, 6].map(|i| &recs[i]).to_vec();
    let a = write(dir2.path(), &mut s2, 0, &first, &[0, 1, 3, 4, 5, 6])?;
    let b = write(dir2.path(), &mut s2, 1, &[&recs[2]], &[2])?;
    for segs in [one, vec![a, b]] {
        let src = SegmentSource::new(MemorySource::new(&recs), segs);
        let n = segs_len(&src);
        assert_eq!(
            src.agg_author_day(i64::MIN..i64::MAX).collect::<Vec<_>>(),
            mem.agg_author_day(i64::MIN..i64::MAX).collect::<Vec<_>>(),
            "{n} segments"
        );
        for prefix in ["", "a", "zz"] {
            assert_eq!(
                src.agg_path_author(prefix).collect::<Vec<_>>(),
                mem.agg_path_author(prefix).collect::<Vec<_>>(),
            );
        }
        for p in 0..mem.paths() {
            let k = Posting::Path(crate::feed::PathId(u32::try_from(p).unwrap_or(0)));
            assert_eq!(src.postings(k).collect::<Vec<_>>(), mem.postings(k).collect::<Vec<_>>());
        }
        for i in 0..mem.idents() {
            let k = Posting::Author(crate::feed::IdentId(u32::try_from(i).unwrap_or(0)));
            assert_eq!(src.postings(k).collect::<Vec<_>>(), mem.postings(k).collect::<Vec<_>>());
        }
        assert_eq!(src.rename_edges().collect::<Vec<_>>(), mem.rename_edges().collect::<Vec<_>>());
    }
    Ok(())
}

/// The builder (spilled path postings) writes the same bytes as the in-memory tables,
/// whether runs were cut or not.
#[test]
fn builder_bytes_equal_table_bytes() {
    let recs = history();
    let ranks: Vec<u32> = (0..7).collect();
    let mut want: Vec<(u16, u32, Vec<u8>)> = Vec::new();
    let (mut a, mut p) = (Aggregates::default(), Postings::default());
    let mut s = Strings::default();
    for r in &recs {
        let (who, paths) = s.ids(r);
        let olds = crate::seg::canon::olds(&s, r);
        a.push(r, who, &paths, &olds);
        p.push(&s, r, who, &paths);
    }
    assert!(
        write_tables(&a, &p, &ranks, s.paths.list.len(), &mut |x, n, b| {
            want.push((x, n, b.to_vec()));
            Ok(())
        })
        .is_ok()
    );
    for limit in [1, 2, usize::MAX] {
        let mut b = super::SegmentBuilder::with_spill(crate::seg::spill::Spill::new(
            limit,
            tempfile::tempfile,
        ));
        let mut s = Strings::default();
        for (r, &k) in recs.iter().zip(&ranks) {
            assert!(b.push(&mut s, r, k).is_ok());
        }
        let mut got: Vec<(u16, u32, Vec<u8>)> = Vec::new();
        assert!(
            b.finish(s.paths.list.len(), &mut |x, n, bytes| {
                got.push((x, n, bytes.to_vec()));
                Ok(())
            })
            .is_ok()
        );
        assert_eq!(got, want, "limit {limit}");
    }
    let _ = super::SegmentBuilder::new(tempfile::tempfile);
}

fn segs_len<B>(s: &SegmentSource<B>) -> usize {
    s.segments.len()
}

/// A path's hits through the block index equal a scan of every block, with the section
/// spread over many blocks and one list longer than a chunk, so it continues across
/// blocks under the same id. Red when the lookup reads only the blocks starting at `want`.
#[test]
fn path_postings_through_the_block_index_equal_a_full_scan() -> SegResult<()> {
    use crate::seg::postings::{Hit, decode_path_postings};
    let paths = 200_000u32;
    let mut by_path: Vec<Vec<Hit>> = (0..paths)
        .map(|p| (0..(p % 7)).map(|i| Hit { ordinal: p * 8 + i, row: i }).collect())
        .collect();
    // One long list in the middle: more than a chunk, so it spans blocks.
    by_path[12_345] = (0..1_500_000u32).map(|i| Hit { ordinal: 1_700_000 + i, row: 0 }).collect();
    let post = Postings::from_parts(by_path, Vec::new(), Vec::new());
    let ranks: Vec<u32> = (0..3_200_000u32).collect();
    let dir = tempfile::tempdir()?;
    let name = "seg-1-0.gts";
    let mut w = ContainerWriter::new(dir.path(), name, Kind::Segment, StringBase::default())?;
    write_tables(&Aggregates::default(), &post, &ranks, 0, &mut |s, n, b| w.block(s, n, b))?;
    w.finish()?;
    let seg = Container::open(dir.path(), name, Kind::Segment)?;
    let blocks = seg.blocks(crate::seg::format::section::PATH_POSTINGS).len();
    assert!(blocks >= 3, "the fixture must spread over blocks, got {blocks}");
    let mut want: Vec<Vec<(u32, u32)>> = vec![Vec::new(); paths as usize];
    let mut buf = Vec::new();
    for e in seg.blocks(crate::seg::format::section::PATH_POSTINGS) {
        let b = seg.read_block(e, &mut buf)?;
        let mut c = crate::seg::bounds::Cursor::new(b, seg.name(), e.offset);
        decode_path_postings(&mut c, e.items, |id, hits| {
            want[id as usize].extend(hits.iter().map(|h| (h.ordinal, h.row)));
        })?;
    }
    let src = SegmentSource::new(MemorySource::new(&[]), vec![seg]);
    // The first and last path of every block, their neighbours, and the long list.
    let firsts: Vec<u32> = src.path_index()?[0].clone();
    let mut probe: Vec<u32> = vec![0, 12_344, 12_345, 12_346, paths - 1];
    for f in firsts {
        probe.extend([f.saturating_sub(1), f, f + 1]);
    }
    for p in probe.into_iter().filter(|p| *p < paths) {
        let got: Vec<(u32, u32)> = src
            .postings(Posting::Path(crate::feed::PathId(p)))
            .map(|h| (h.rank.0, h.row))
            .collect();
        assert_eq!(got, want[p as usize], "path {p}");
    }
    Ok(())
}

/// Every stored counter, one at a time: the count (commits or touches), then each of the six
/// line sums.
const COUNTERS: usize = 7;

/// Sets counter `k` of a row to `v`.
fn set_counter(count: &mut u64, lines: &mut crate::seg::agg::Sums, k: usize, v: u64) {
    match k {
        0 => *count = v,
        1 => lines.add = v,
        2 => lines.del = v,
        3 => lines.rewritten = v,
        4 => lines.code_add = v,
        5 => lines.code_del = v,
        _ => lines.code_rewritten = v,
    }
}

/// Aggregates of `recs` with counter `k` of every row near `u64::MAX`: what a hostile segment
/// with recomputed crcs decodes to. Two of them sum past u64.
fn near_max(s: &mut Strings, recs: &[&CommitRecord], k: usize) -> (Aggregates, Postings) {
    let (mut a, mut p) = (Aggregates::default(), Postings::default());
    for r in recs {
        let (who, paths) = s.ids(r);
        let olds = crate::seg::canon::olds(s, r);
        a.push(r, who, &paths, &olds);
        p.push(s, r, who, &paths);
    }
    let big = u64::MAX - 1;
    for v in a.author_day.values_mut() {
        set_counter(&mut v.commits, &mut v.lines, k, big);
    }
    for v in a.path_author.values_mut() {
        set_counter(&mut v.touches, &mut v.lines, k, big);
    }
    for v in a.life_month.values_mut() {
        set_counter(&mut v.touches, &mut v.lines, k, big);
    }
    for v in a.life_author.values_mut() {
        set_counter(&mut v.touches, &mut v.lines, k, big);
    }
    (a, p)
}

/// Two segments whose stored counters each sit near `u64::MAX`: summing them (the read of an
/// unmerged set, and the merge) is an unreadable cache, never a panic, a wrapped number or a
/// saturated one. Every counter field of every aggregate table is tried in turn, so a new
/// sum written with a plain `+=` fails here.
#[test]
fn counters_near_max_in_two_segments_are_unreadable() -> SegResult<()> {
    let recs = history();
    let all: Vec<&CommitRecord> = recs.iter().collect();
    let ranks: Vec<u32> = (0..u32::try_from(recs.len()).unwrap_or(0)).collect();
    for k in 0..COUNTERS {
        let dir = tempfile::tempdir()?;
        let mut s = Strings::default();
        let mut segs = Vec::new();
        let mut parts = Vec::new();
        for n in 0..2 {
            let (a, p) = near_max(&mut s, &all, k);
            let name = format!("seg-1-{n}.gts");
            let mut w =
                ContainerWriter::new(dir.path(), &name, Kind::Segment, StringBase::default())?;
            write_tables(&a, &p, &ranks, s.paths.list.len(), &mut |sec, items, b| {
                w.block(sec, items, b)
            })?;
            w.finish()?;
            segs.push(Container::open(dir.path(), &name, Kind::Segment)?);
            parts.push((a, p));
        }
        let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let src = SegmentSource::new(MemorySource::new(&recs), segs);
            let days = src.agg_author_day(i64::MIN..i64::MAX).count();
            let paths = src.agg_path_author("").count();
            (days, paths, src.failure())
        }));
        let (_, _, failure) = got.unwrap_or_else(|_| panic!("counter {k}: the read panicked"));
        let failure =
            failure.unwrap_or_else(|| panic!("counter {k}: a summed-past-u64 read was served"));
        assert!(failure.0.contains(crate::seg::agg::SUM_OVER), "counter {k}: {failure}");
        // The merge of the same two parts.
        let n = crate::seg::merge::Renumber::new(&s, std::iter::empty());
        let merge_parts: Vec<crate::seg::merge::Part<'_>> = parts
            .iter()
            .map(|(a, p)| crate::seg::merge::Part { agg: a, postings: p, rank: &ranks })
            .collect();
        let merged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::seg::merge::aggregates(&merge_parts, &n).map(|_| ())
        }))
        .unwrap_or_else(|_| panic!("counter {k}: the merge panicked"));
        assert!(
            merged.as_ref().is_err_and(crate::seg::SegError::is_unreadable),
            "counter {k}: the merge gave {merged:?}"
        );
    }
    Ok(())
}
