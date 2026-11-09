//! The tip file against the row read it replaces: every per-file page over a set that holds
//! the tip file must equal the same page over the records in memory, byte for byte, through
//! renames (chains, a swap, a rename onto a freed name), a deleted and reused name, merges,
//! mailmap, bots and whitespace, every period, a selection, and sets built in one step or by
//! appends that rename old files.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::feed::{DiskSource, InsightsSource, MemorySource, SegmentSource, Stream};
use crate::insights::HeadFile;
use crate::seg::format::Kind;
use crate::seg::manifest::Manifest;
use crate::seg::read::Store;
use crate::{CodeStat, CommitRecord, CommitSource, FileStat, Index, Insights, Options, Period};

const DAY: i64 = 86_400;
/// 2024-03-31 23:30 UTC: a month edge in UTC, and past midnight in +01:00.
const EDGE: i64 = 1_711_927_800;

#[derive(Default)]
struct Hist {
    recs: RefCell<HashMap<String, CommitRecord>>,
    order: RefCell<Vec<String>>,
}

fn id(n: usize) -> String {
    format!("{n:040x}")
}

impl CommitSource for Hist {
    fn rev_list(&self, _rev: &str) -> io::Result<Vec<String>> {
        Ok(self.order.borrow().clone())
    }

    fn resolve(&self, _rev: &str) -> Option<String> {
        self.order.borrow().last().cloned()
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        Ok(self.recs.borrow()[id].clone())
    }
}

fn f(path: &str, add: u32, del: u32) -> FileStat {
    FileStat {
        path: path.into(),
        additions: add,
        deletions: del,
        lines: add + 1,
        ..FileStat::default()
    }
}

fn mv(old: &str, new: &str, add: u32) -> FileStat {
    FileStat {
        old: Some(old.into()),
        ..f(new, add, 0)
    }
}

fn gone(path: &str) -> FileStat {
    FileStat {
        deleted: true,
        ..f(path, 0, 3)
    }
}

fn ws(path: &str) -> FileStat {
    FileStat {
        code: Some(CodeStat {
            additions: 1,
            deletions: 0,
            rewritten: 0,
        }),
        ..f(path, 6, 4)
    }
}

/// Commit `n` by `who` at `time` (offset seconds east), on the previous commit, or a merge of
/// the previous two when `merge`.
fn commit(h: &Hist, n: usize, who: &str, time: i64, offset: i32, files: Vec<FileStat>) {
    let parents = if n == 0 { vec![] } else { vec![id(n - 1)] };
    let r = CommitRecord {
        id: id(n),
        author: who.into(),
        email: format!("{}@x.io", who.to_lowercase().replace(' ', ".")),
        time,
        offset,
        parents,
        files,
        ..CommitRecord::default()
    };
    h.recs.borrow_mut().insert(r.id.clone(), r);
    h.order.borrow_mut().push(id(n));
}

fn merge(h: &Hist, n: usize, time: i64, files: Vec<FileStat>) {
    commit(h, n, "Ann", time, 0, files);
    let mut recs = h.recs.borrow_mut();
    let r = recs.get_mut(&id(n)).expect("just added");
    r.merge = true;
    r.parents.push(id(n.saturating_sub(2)));
}

/// The rename-heavy history, in steps so a test can append it in parts. Each step is a list
/// of commits; the tip after all of them holds the files of [`head`].
fn steps() -> Vec<Vec<Step>> {
    let c = |who: &str, d: i64, o: i32, files: Vec<FileStat>| (who.to_owned(), d, o, files, false);
    let m = |d: i64, files: Vec<FileStat>| (String::new(), d, 0, files, true);
    vec![
        vec![
            c(
                "Ann",
                -900,
                0,
                vec![f("a.rs", 10, 0), f("b.rs", 4, 0), f("keep.rs", 3, 0)],
            ),
            c(
                "Bob",
                -800,
                3600,
                vec![f("a.rs", 2, 1), f("old/x.py", 7, 0)],
            ),
            c(
                "dependabot[bot]",
                -700,
                0,
                vec![f("keep.rs", 1, 1), f("lock.json", 90, 0)],
            ),
            c("Ann", -600, -18_000, vec![ws("b.rs"), f("tmp.rs", 5, 0)]),
            m(-590, vec![f("a.rs", 2, 2), f("b.rs", 1, 0)]),
            c("Cy", -500, 0, vec![gone("tmp.rs")]),
            c(
                "Ann Other",
                -400,
                0,
                vec![f("tmp.rs", 9, 0), f("keep.rs", 2, 0)],
            ),
        ],
        vec![
            // Chain: old/x.py to mid/x.py to new/x.py, and a swap a <-> b.
            c("Bob", -300, 0, vec![mv("old/x.py", "mid/x.py", 1)]),
            c(
                "Ann",
                -200,
                0,
                vec![mv("a.rs", "t.rs", 0), mv("b.rs", "a.rs", 1)],
            ),
            c(
                "Ann",
                -199,
                0,
                vec![mv("t.rs", "b.rs", 0), f("keep.rs", 1, 0)],
            ),
            m(-150, vec![mv("keep.rs", "kept.rs", 0)]),
        ],
        vec![
            c(
                "Bob",
                -100,
                0,
                vec![mv("mid/x.py", "new/x.py", 2), f("a.rs", 1, 1)],
            ),
            // Rename onto a freed name: tmp.rs is deleted, then lock.json moves onto it.
            c("Cy", -60, 0, vec![gone("tmp.rs")]),
            c("Cy", -59, 0, vec![mv("lock.json", "tmp.rs", 0)]),
            // Around a month edge in UTC and in +01:00 and -05:00.
            c("Ann", 0, 0, vec![f("new/x.py", 1, 0), f("b.rs", 2, 0)]),
            c(
                "Bob",
                0,
                3600,
                vec![f("new/x.py", 1, 1), f("keep.rs", 4, 0)],
            ),
            c("Cy", 0, -18_000, vec![f("a.rs", 3, 0), ws("b.rs")]),
            c("Ann", 1, 0, vec![f("a.rs", 1, 0)]),
        ],
    ]
}

/// One commit of a step: author, quarter days from the edge, offset, rows, merge.
type Step = (String, i64, i32, Vec<FileStat>, bool);

/// Plays one step into `h`, numbering on from `*n`.
fn play(h: &Hist, n: &mut usize, step: &[Step]) {
    for (who, d, o, files, is_merge) in step {
        let t = EDGE + d * DAY / 4;
        if *is_merge {
            merge(h, *n, t, files.clone());
        } else {
            commit(h, *n, who, t, *o, files.clone());
        }
        *n += 1;
    }
}

fn head() -> Vec<HeadFile> {
    [
        ("a.rs", 12),
        ("b.rs", 9),
        ("kept.rs", 7),
        ("keep.rs", 5),
        ("new/x.py", 11),
        ("tmp.rs", 90),
        ("missing.rs", 4),
    ]
    .iter()
    .map(|(p, l)| HeadFile {
        path: (*p).into(),
        lines: *l,
    })
    .collect()
}

/// Head files padded past the per-path lookup, so the many-path read is the one compared.
fn many() -> Vec<HeadFile> {
    let mut v = head();
    v.extend((0..80).map(|i| HeadFile {
        path: format!("pad/{i}.rs"),
        lines: 1,
    }));
    v
}

fn disk(dir: &Path) -> SegmentSource<DiskSource> {
    let store = Store::open(dir).expect("open").expect("a set");
    let disk = DiskSource::open(store, &mut |p| Ok(p.to_vec())).expect("disk");
    let tables = Store::open(dir).expect("open").expect("set");
    SegmentSource::new(disk, tables.segments)
}

fn tip_files(dir: &Path) -> usize {
    let m = Manifest::load(dir).expect("manifest").expect("set");
    m.live.iter().filter(|f| f.kind == Kind::Tip as u8).count()
}

/// Every per-file page under `opts`, as one string.
fn pages(ins: &Insights<'_>, files: &[HeadFile]) -> String {
    let mut out: Vec<String> = Vec::new();
    for p in [
        Period::All,
        Period::Year,
        Period::Quarter,
        Period::Month,
        Period::Week,
    ] {
        out.push(format!("{p:?}\n{:?}", ins.hotspots(files, p)));
        out.push(format!("{:?}", ins.ownership(files, p, 1)));
        out.push(format!("{:?}", ins.coupling(files, p, 1, 50)));
    }
    out.push(format!("{:?}", ins.code_age(files)));
    let tags = [("v1".to_owned(), id(3)), ("v2".to_owned(), id(9))];
    let cadence = ins.in_period(Period::All, |c| ins.cadence_in(&tags, c));
    out.push(format!("{cadence:?}"));
    out.join("\n")
}

fn option_sets() -> Vec<Options> {
    vec![
        Options::default(),
        Options {
            include_bots: true,
            count_whitespace: true,
            ..Options::default()
        },
        Options {
            include_merges: true,
            ..Options::default()
        },
    ]
}

/// The set in `dir` answers every page as the records do.
fn same_as_records(dir: &Path, h: &Hist, case: &str) {
    let order = h.order.borrow().clone();
    let recs: Vec<CommitRecord> = order.iter().map(|i| h.recs.borrow()[i].clone()).collect();
    let d = disk(dir);
    assert!(d.has_tip(), "{case}: no current tip file");
    assert_eq!(tip_files(dir), 1, "{case}: one live tip file");
    let mem = MemorySource::new(&recs);
    for opts in option_sets() {
        for files in [head(), many()] {
            let a = pages(&Insights::from_source(&d, opts.clone(), None), &files);
            let b = pages(&Insights::from_source(&mem, opts.clone(), None), &files);
            assert_eq!(a, b, "{case}: {opts:?}");
        }
    }
    assert!(d.failure().is_none(), "{case}: the tip read failed");
}

fn index(dir: &Path, h: &Hist) {
    Index::open_dir(dir.to_path_buf())
        .expect("index")
        .update(h, "HEAD", &|_| {})
        .expect("update");
}

/// One build of the whole history.
#[test]
fn a_build_answers_every_page_as_the_rows_do() {
    let h = Hist::default();
    let mut n = 0;
    for s in steps() {
        play(&h, &mut n, &s);
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("gitterrace");
    index(&dir, &h);
    same_as_records(&dir, &h, "build");
}

/// Appends whose new commits rename old files: the old items move to the new tip names, and
/// the set answers as a fresh build and as the rows.
#[test]
fn appends_that_rename_old_files_answer_as_the_rows_do() {
    let h = Hist::default();
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("gitterrace");
    let mut n = 0;
    for (i, s) in steps().iter().enumerate() {
        play(&h, &mut n, s);
        index(&dir, &h);
        same_as_records(&dir, &h, &format!("after step {i}"));
    }
    // One commit at a time, each an append.
    let one = Hist::default();
    let tmp1 = tempfile::tempdir().expect("tmp");
    let dir1 = tmp1.path().join("gitterrace");
    let mut m = 0;
    for s in steps() {
        for c in s {
            play(&one, &mut m, &[c]);
            index(&dir1, &one);
        }
    }
    same_as_records(&dir1, &one, "one commit at a time");
}

/// A tip file written for another set is not read, and the next update writes a current one;
/// a crash before the new manifest leaves the old set readable with its own tip file.
#[test]
fn a_stale_tip_file_is_not_read() {
    let h = Hist::default();
    let mut n = 0;
    for s in steps() {
        play(&h, &mut n, &s);
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("gitterrace");
    index(&dir, &h);
    let mut m = Manifest::load(&dir).expect("manifest").expect("set");
    m.fingerprint ^= 1;
    m.commit(&dir).expect("commit");
    assert!(!disk(&dir).has_tip());
    super::sync(&dir).expect("sync");
    assert!(disk(&dir).has_tip());
}

/// Hostile item bytes behind a valid crc are refused, never served.
#[test]
fn hostile_tip_items_are_unreadable() {
    let check = |b: &[u8], paths: usize| {
        let mut cur = crate::seg::bounds::Cursor::new(b, "t", 0);
        super::item(&mut cur, paths, &mut Vec::new())
    };
    assert!(check(&[1, 0, 5, 0], 1).is_ok());
    assert!(check(&[1, 3, 5, 0], 1).is_err_and(|e| e.is_unreadable()));
    assert!(check(&[1, 0, 5], 1).is_err_and(|e| e.is_unreadable()));
    // Code churn above the raw one past u32 or below zero.
    assert!(check(&[1, 0, 1, 4], 1).is_err_and(|e| e.is_unreadable()));
    assert!(check(&[2, 0, 1, 0], 1).is_err_and(|e| e.is_unreadable()));
}

/// A source that counts the file rows `rows_for` hands out and the commit scans, forwarding
/// the rest.
struct Counting<S> {
    inner: S,
    rows: std::sync::atomic::AtomicUsize,
    /// Commit column scans (`commits_in` calls).
    scans: std::sync::atomic::AtomicUsize,
}

impl<S: InsightsSource> Counting<S> {
    fn take(&self) -> usize {
        self.rows.swap(0, std::sync::atomic::Ordering::Relaxed)
    }
}

macro_rules! forward {
    ($($name:ident($($a:ident: $t:ty),*) -> $r:ty;)*) => {
        $(fn $name(&self, $($a: $t),*) -> $r { self.inner.$name($($a),*) })*
    };
}

impl<S: InsightsSource> InsightsSource for Counting<S> {
    forward! {
        len() -> usize;
        commit(r: crate::feed::Rank) -> Option<crate::feed::CommitRow>;
        commit_id(r: crate::feed::Rank) -> Option<std::sync::Arc<str>>;
        parent_ids(r: crate::feed::Rank) -> Vec<std::sync::Arc<str>>;
        find_commit(id: &str) -> Result<Option<crate::feed::Rank>, crate::feed::SourceError>;
        agg_author_day(d: std::ops::Range<i64>) -> Stream<'_, crate::feed::AuthorDayRow>;
        agg_path_author(p: &str) -> crate::feed::Stream<'_, crate::feed::PathAuthorRow>;
        postings(k: crate::feed::Posting) -> crate::feed::Stream<'_, crate::feed::Hit>;
        rename_edges() -> crate::feed::Stream<'_, crate::feed::RenameEdge>;
        path(id: crate::feed::PathId) -> Option<std::sync::Arc<str>>;
        find_path(p: &str) -> Result<Option<crate::feed::PathId>, crate::feed::SourceError>;
        paths() -> usize;
        ident(id: crate::feed::IdentId) -> Option<(std::sync::Arc<str>, std::sync::Arc<str>)>;
        idents() -> usize;
        label(id: crate::feed::LabelId) -> Option<std::sync::Arc<str>>;
        find_label(n: &str) -> Result<Option<crate::feed::LabelId>, crate::feed::SourceError>;
        failure() -> Option<crate::feed::SourceError>;
        has_tip() -> bool;
    }

    fn commits_in(
        &self,
        r: crate::feed::CommitRange,
    ) -> crate::feed::Stream<'_, crate::feed::CommitRow> {
        self.scans
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.commits_in(r)
    }

    fn rows_for(
        &self,
        c: Vec<crate::feed::Rank>,
    ) -> crate::feed::Stream<'_, (crate::feed::Rank, Vec<crate::feed::FileRow>)> {
        Box::new(self.inner.rows_for(c).inspect(|(_, r)| {
            self.rows
                .fetch_add(r.len(), std::sync::atomic::Ordering::Relaxed);
        }))
    }

    fn tip_touches(&self, each: &mut dyn FnMut(crate::feed::Rank, &[super::TipTouch])) -> bool {
        self.inner.tip_touches(each)
    }
}

/// The per-file pages over many head files read no file row once the tip file is there, in
/// every period: a change that reads the rows again (the pages' cost on a large history)
/// fails here. Red on the row read, which reads every row of every kept commit.
#[test]
fn per_file_pages_read_no_file_rows_with_a_tip_file() {
    let h = Hist::default();
    let mut n = 0;
    for s in steps() {
        play(&h, &mut n, &s);
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("gitterrace");
    index(&dir, &h);
    let src = Counting {
        inner: disk(&dir),
        rows: std::sync::atomic::AtomicUsize::new(0),
        scans: std::sync::atomic::AtomicUsize::new(0),
    };
    let ins = Insights::from_source(&src, Options::default(), None);
    let files = many();
    // Warm the derived tables, which read rows of their own once.
    drop(ins.hotspots(&files, Period::All));
    src.take();
    for p in [Period::All, Period::Year, Period::Month] {
        drop(ins.hotspots(&files, p));
        assert_eq!(src.take(), 0, "hotspots {p:?}");
        drop(ins.ownership(&files, p, 1));
        assert_eq!(src.take(), 0, "ownership {p:?}");
        drop(ins.coupling(&files, p, 1, 50));
        assert_eq!(src.take(), 0, "coupling {p:?}");
    }
    drop(ins.code_age(&files));
    assert_eq!(src.take(), 0, "code age");
    // Guard class: a whole-history scan a section pays without needing it. Release cadence
    // with no tag reads no commit column.
    src.scans.store(0, std::sync::atomic::Ordering::Relaxed);
    let none = ins.in_period(Period::All, |c| ins.cadence_in(&[], c));
    assert!(none.releases.is_empty());
    let scans = src.scans.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(scans, 0, "cadence without tags scanned the commits");
    // The same pages without the tip file read the rows: the guard can see a row read.
    let mem_recs: Vec<CommitRecord> = h
        .order
        .borrow()
        .iter()
        .map(|i| h.recs.borrow()[i].clone())
        .collect();
    let mem = Counting {
        inner: MemorySource::new(&mem_recs),
        rows: std::sync::atomic::AtomicUsize::new(0),
        scans: std::sync::atomic::AtomicUsize::new(0),
    };
    let ins = Insights::from_source(&mem, Options::default(), None);
    drop(ins.hotspots(&files, Period::All));
    mem.take();
    drop(ins.hotspots(&files, Period::All));
    assert!(mem.take() > 0, "the row read reads rows");
}
