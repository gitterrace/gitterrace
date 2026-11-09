//! The skeleton over a real segment set: from the chain the index stored, and from the walk
//! that reads the records, the page gets the same lines, ids, times and sums.

use std::cell::RefCell;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use gitterrace_index::feed::{self, InsightsSource};
use gitterrace_index::{CommitRecord, CommitSource, FileStat, Index, Options, PathFilter};

use crate::languages::chain::tests::Reads;
use crate::languages::chain::{Skeleton, wanted};
use crate::languages::{Held, View};

/// Records the test hands the index, oldest first.
struct Recs(RefCell<Vec<CommitRecord>>);

impl CommitSource for Recs {
    fn rev_list(&self, _rev: &str) -> io::Result<Vec<String>> {
        Ok(self.0.borrow().iter().map(|r| r.id.clone()).collect())
    }

    fn resolve(&self, _rev: &str) -> Option<String> {
        self.0.borrow().last().map(|r| r.id.clone())
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        self.0
            .borrow()
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or_else(|| io::Error::other("no such commit"))
    }
}

fn file(path: &str, add: u32, del: u32) -> FileStat {
    FileStat {
        path: Arc::from(path),
        additions: add,
        deletions: del,
        ..FileStat::default()
    }
}

/// `n` first-parent commits with a merged side commit every seventh, a rename across
/// languages (`old.c` to `new.rs`), a binary file, a deleted path, a lock file the filter
/// skips and enough languages that `Other` folds.
fn history(n: usize) -> Vec<CommitRecord> {
    const EXT: [&str; 10] = ["rs", "py", "go", "c", "js", "ts", "rb", "md", "sh", "toml"];
    let id = |i: usize| format!("{i:040x}");
    let mut out = Vec::new();
    for i in 0..n {
        let ext = EXT[(i * 7) % EXT.len()];
        let add = u32::try_from((i * 31) % 97 + 1).unwrap_or(1);
        let mut files = vec![file(&format!("src/f{}.{ext}", i % 5), add, add / 3)];
        let mut parents = if i == 0 { vec![] } else { vec![id(i - 1)] };
        match i {
            3 => files.push(file("old.c", 40, 0)),
            9 => {
                let mut renamed = file("new.rs", 40, 0);
                renamed.old = Some(Arc::from("old.c"));
                files.extend([file("old.c", 0, 40), renamed]);
            }
            11 => files.push(FileStat {
                binary: true,
                ..file("logo.png", 0, 0)
            }),
            12 => files.push(file("gone.py", 25, 0)),
            20 => files.push(FileStat {
                deleted: true,
                ..file("gone.py", 0, 25)
            }),
            _ => files.push(file("Cargo.lock", 9, 2)),
        }
        // Every seventh commit merges a side commit made just before it.
        if i % 7 == 6 {
            let side = n + i;
            out.push(CommitRecord {
                id: id(side),
                author: "s".into(),
                email: "s@x".into(),
                time: 1_600_000_000 + i64::try_from(i).unwrap_or(0) * 3600 - 60,
                parents: parents.clone(),
                files: vec![file("side.go", 50, 0)],
                ..CommitRecord::default()
            });
            parents.push(id(side));
        }
        out.push(CommitRecord {
            id: id(i),
            author: "a".into(),
            email: "a@x.io".into(),
            time: 1_600_000_000 + i64::try_from(i).unwrap_or(0) * 3600,
            merge: parents.len() > 1,
            parents,
            files,
            ..CommitRecord::default()
        });
    }
    out
}

fn filter() -> PathFilter {
    PathFilter::new(&Options::default(), None)
}

/// A set built by the index, served the way pages read it.
fn set(recs: Vec<CommitRecord>) -> (tempfile::TempDir, Arc<dyn InsightsSource>) {
    let tmp = tempfile::tempdir().expect("tmp");
    let idx = Index::open_dir(tmp.path().join("c")).expect("index");
    let src = Recs(RefCell::new(recs));
    idx.update(&src, "HEAD", &|_| {}).expect("update");
    let set = idx
        .disk_source(&src, "HEAD")
        .expect("open")
        .expect("the set holds HEAD");
    (tmp, set.erased().unchecked())
}

/// The source without its stored chain, so the skeleton walks.
struct Walk(Arc<dyn InsightsSource>);

fn view(range: Option<(usize, usize)>) -> View {
    View {
        time: false,
        share: false,
        range,
        picked: range.is_some(),
        period: gitterrace_index::Period::All,
        dflt: gitterrace_index::Period::All,
        held: Held::Commits,
    }
}

/// From the stored chain and from the walk, every view reads the same lines, ids, times and
/// range sums: through the rename across languages, the binary, the deletion and the merges.
#[test]
fn the_stored_chain_gives_the_walks_skeleton() {
    let (_t, src) = set(history(600));
    assert!(src.chain_len().is_some(), "the index stored the chain");
    let f = filter();
    let stored = Arc::new(Skeleton::build(&*src, &f));
    let walked = Arc::new(Skeleton::build(&Walk(Arc::clone(&src)), &f));
    let n = stored.len();
    assert_eq!(n, walked.len());
    assert_eq!(n, 600, "side commits are not on the chain");
    assert_eq!(stored.langs, walked.langs);
    assert!(
        stored.langs.iter().any(|l| l == "Other"),
        "the fixture folds"
    );
    assert!(src.failure().is_none());
    for range in [
        None,
        Some((70, 90)),
        Some((10, 500)),
        Some((0, n - 1)),
        Some((1, 2)),
    ] {
        let a = stored.view(&*src, &f, view(range));
        let b = walked.view(&*src, &f, view(range));
        for i in wanted(n, range) {
            assert_eq!(a.lines(i), b.lines(i), "lines at {i} for {range:?}");
            assert_eq!(a.id(i), b.id(i), "id at {i} for {range:?}");
            assert_eq!(a.time(i), b.time(i), "time at {i} for {range:?}");
            assert_eq!(a.offset(i), b.offset(i), "offset at {i} for {range:?}");
        }
        assert_eq!((a.added, a.removed), (b.added, b.removed), "{range:?}");
    }
    for day in [0, 18_500, 18_520, 30_000] {
        assert_eq!(
            stored.first_from_day(&*src, day),
            walked.first_from_day(&Walk(Arc::clone(&src)), day),
            "first position from day {day}"
        );
    }
}

/// The build from the stored chain reads no commit record and does no oid lookup: the cold
/// page's cost is one sequential read of the chain file, whatever the history length.
#[test]
fn the_build_reads_no_commit_record() {
    for n in [200, 2000] {
        let (_t, src) = set(history(n));
        let src = Reads {
            inner: &*src,
            lookups: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
        };
        let sk = Skeleton::build(&src, &filter());
        let (lookups, records) = src.take();
        assert_eq!(sk.len(), n);
        assert_eq!(
            (lookups, records),
            (0, 0),
            "reads building over {n} commits"
        );
        sk.first_from_day(&src, 0);
        assert_eq!(
            src.take(),
            (0, 0),
            "reads finding a period over {n} commits"
        );
    }
}

/// A filter change needs no rebuild: the stored rows are keyed by path, so a new exclude
/// gives the walk's lines at once.
#[test]
fn an_exclude_applies_without_a_rebuild() {
    let (_t, src) = set(history(300));
    let opts = Options {
        exclude: vec!["*.py".to_owned(), "src/f1.*".to_owned()],
        ..Options::default()
    };
    let f = PathFilter::new(&opts, None);
    let stored = Skeleton::build(&*src, &f);
    let walked = Skeleton::build(&Walk(Arc::clone(&src)), &f);
    assert_eq!(stored.langs, walked.langs);
    assert!(!stored.langs.iter().any(|l| l == "Python"));
    let (a, b) = (Arc::new(stored), Arc::new(walked));
    let (va, vb) = (a.view(&*src, &f, view(None)), b.view(&*src, &f, view(None)));
    for i in wanted(a.len(), None) {
        assert_eq!(va.lines(i), vb.lines(i), "lines at {i}");
    }
}

impl InsightsSource for Walk {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn commit(&self, r: feed::Rank) -> Option<feed::CommitRow> {
        self.0.commit(r)
    }
    fn commits_in(&self, r: feed::CommitRange) -> feed::Stream<'_, feed::CommitRow> {
        self.0.commits_in(r)
    }
    fn rows_for(&self, c: Vec<feed::Rank>) -> feed::Stream<'_, (feed::Rank, Vec<feed::FileRow>)> {
        self.0.rows_for(c)
    }
    fn commit_id(&self, r: feed::Rank) -> Option<Arc<str>> {
        self.0.commit_id(r)
    }
    fn parent_ids(&self, r: feed::Rank) -> Vec<Arc<str>> {
        self.0.parent_ids(r)
    }
    fn find_commit(&self, id: &str) -> Result<Option<feed::Rank>, feed::SourceError> {
        self.0.find_commit(id)
    }
    fn agg_author_day(&self, d: std::ops::Range<i64>) -> feed::Stream<'_, feed::AuthorDayRow> {
        self.0.agg_author_day(d)
    }
    fn agg_path_author(&self, p: &str) -> feed::Stream<'_, feed::PathAuthorRow> {
        self.0.agg_path_author(p)
    }
    fn postings(&self, k: feed::Posting) -> feed::Stream<'_, feed::Hit> {
        self.0.postings(k)
    }
    fn rename_edges(&self) -> feed::Stream<'_, feed::RenameEdge> {
        self.0.rename_edges()
    }
    fn path(&self, id: feed::PathId) -> Option<Arc<str>> {
        self.0.path(id)
    }
    fn find_path(&self, p: &str) -> Result<Option<feed::PathId>, feed::SourceError> {
        self.0.find_path(p)
    }
    fn paths(&self) -> usize {
        self.0.paths()
    }
    fn ident(&self, id: feed::IdentId) -> Option<(Arc<str>, Arc<str>)> {
        self.0.ident(id)
    }
    fn idents(&self) -> usize {
        self.0.idents()
    }
    fn label(&self, id: feed::LabelId) -> Option<Arc<str>> {
        self.0.label(id)
    }
    fn find_label(&self, n: &str) -> Result<Option<feed::LabelId>, feed::SourceError> {
        self.0.find_label(n)
    }
    fn failure(&self) -> Option<feed::SourceError> {
        self.0.failure()
    }
}

/// A path whose net lines go negative over the chain (removed in a shallow history whose
/// adding commit is not there) and a path added then deleted (net 0) stay out of the
/// legend from the stored chain exactly as from the walk, and every line still sums.
#[test]
fn negative_and_net_zero_paths_stay_hidden_as_the_walk_hides_them() {
    let id = |i: usize| format!("{i:040x}");
    let commit = |i: usize, files: Vec<FileStat>| CommitRecord {
        id: id(i),
        author: "a".into(),
        email: "a@x.io".into(),
        time: 1_600_000_000 + i64::try_from(i).unwrap_or(0) * 3600,
        parents: if i == 0 { vec![] } else { vec![id(i - 1)] },
        files,
        ..CommitRecord::default()
    };
    let gone = FileStat {
        deleted: true,
        ..file("gone.kt", 0, 5)
    };
    let mut recs = vec![
        commit(0, vec![file("a.rs", 10, 0), file("gone.kt", 5, 0)]),
        commit(1, vec![gone, file("old.go", 0, 7), file("b.py", 20, 0)]),
    ];
    // Past one mark gap, so the negative net crosses a mark too.
    recs.extend((2..150).map(|i| commit(i, vec![file("a.rs", 1, 1)])));
    let (_t, src) = set(recs);
    assert!(src.chain_len().is_some(), "the index stored the chain");
    let f = filter();
    let stored = Arc::new(Skeleton::build(&*src, &f));
    let walked = Arc::new(Skeleton::build(&Walk(Arc::clone(&src)), &f));
    assert_eq!(stored.langs, walked.langs);
    for hidden in ["Go", "Kotlin"] {
        assert!(!stored.langs.iter().any(|l| l == hidden), "{hidden} shows");
    }
    let n = stored.len();
    for range in [None, Some((0, 1)), Some((1, n - 1))] {
        let a = stored.view(&*src, &f, view(range));
        let b = walked.view(&*src, &f, view(range));
        for i in wanted(n, range) {
            assert_eq!(a.lines(i), b.lines(i), "lines at {i} for {range:?}");
        }
        assert_eq!((a.added, a.removed), (b.added, b.removed), "{range:?}");
    }
    let last = stored.view(&*src, &f, view(None)).lines(n - 1);
    assert_eq!(
        last.iter().sum::<i64>(),
        10 + 20 - 7,
        "no line is lost or wrapped"
    );
    assert!(src.failure().is_none());
}

/// The stored chain keeps no cut points: blocks split where the bytes fill, and the page
/// lays its marks over positions as it reads. A chain spanning several blocks, with mark
/// gaps across every block edge, and chains of fewer than one gap and of an exact multiple
/// of it, give the walk's skeleton.
#[test]
fn marks_across_block_edges_give_the_walks_skeleton() {
    let id = |i: usize| format!("{i:040x}");
    // Wide commits so the chain fills more than one 1 MiB block.
    let wide = |i: usize, n: usize| CommitRecord {
        id: id(i),
        author: "a".into(),
        email: "a@x.io".into(),
        time: 1_600_000_000 + i64::try_from(i).unwrap_or(0) * 3600,
        parents: if i == 0 { vec![] } else { vec![id(i - 1)] },
        files: (0..n)
            .map(|k| file(&format!("d{}/f{k}.{}", i % 3, ["rs", "py"][k % 2]), 3, 1))
            .collect(),
        ..CommitRecord::default()
    };
    for (n, width) in [(300, 2500), (40, 2), (128, 2)] {
        let (tmp, src) = set((0..n).map(|i| wide(i, width)).collect());
        let store = gitterrace_index::seg::read::Store::open(&tmp.path().join("c"))
            .expect("open")
            .expect("a set");
        let blocks = store
            .chain
            .as_ref()
            .expect("chain")
            .blocks(gitterrace_index::seg::format::section::CHAIN)
            .len();
        assert!(
            width < 100 || blocks > 1,
            "{blocks} blocks over {n} wide steps"
        );
        let f = filter();
        let stored = Arc::new(Skeleton::build(&*src, &f));
        let walked = Arc::new(Skeleton::build(&Walk(Arc::clone(&src)), &f));
        assert_eq!(stored.len(), n);
        for range in [None, Some((0, n - 1)), Some((n / 3, n / 2)), Some((63, 65))] {
            let range = range.filter(|r| r.1 < n);
            let a = stored.view(&*src, &f, view(range));
            let b = walked.view(&*src, &f, view(range));
            for i in wanted(n, range) {
                assert_eq!(a.lines(i), b.lines(i), "lines at {i} of {n}, {range:?}");
            }
            assert_eq!((a.added, a.removed), (b.added, b.removed), "{n}, {range:?}");
        }
        assert!(src.failure().is_none());
    }
}
