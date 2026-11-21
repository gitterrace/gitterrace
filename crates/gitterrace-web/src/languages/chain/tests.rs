use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitterrace_index::feed::{self, InsightsSource, MemorySource, Rank};
use gitterrace_index::{CommitRecord, FileStat, Options, PathFilter};

use super::build::Tally;
use super::{Fold, Skeleton, State, walk, wanted};
use crate::languages::{Held, View};

/// A linear history of `n` commits over a handful of languages, with deletions, a lock file
/// the filter skips and more languages than the page shows, so `Other` is folded.
fn history(n: usize) -> Vec<CommitRecord> {
    const EXT: [&str; 11] = ["rs", "py", "go", "c", "js", "ts", "rb", "md", "sh", "toml", "lock"];
    (0..n)
        .map(|i| {
            let ext = EXT[(i * 7) % EXT.len()];
            let add = u32::try_from((i * 31) % 97 + 1).unwrap_or(1);
            let del = u32::try_from((i * 13) % 23).unwrap_or(0).min(add / 2);
            CommitRecord {
                id: format!("{i:040x}"),
                author: "a".to_owned(),
                email: "a@x.io".to_owned(),
                time: 1_600_000_000 + i64::try_from(i).unwrap_or(0) * 3600,
                parents: if i == 0 { vec![] } else { vec![format!("{:040x}", i - 1)] },
                files: vec![FileStat {
                    path: Arc::from(format!("src/f{}.{ext}", i % 5).as_str()),
                    additions: add,
                    deletions: del,
                    ..FileStat::default()
                }],
                ..CommitRecord::default()
            }
        })
        .collect()
}

/// Lines after every commit, ids and times, the way the page computed them before.
struct Reference {
    ids: Vec<String>,
    times: Vec<i64>,
    lines: Vec<Vec<i64>>,
    adds: Vec<u64>,
    dels: Vec<u64>,
}

fn reference(src: &dyn InsightsSource, filter: &PathFilter) -> Reference {
    let newest = src
        .len()
        .checked_sub(1)
        .map(|i| gitterrace_index::feed::Rank(u32::try_from(i).unwrap_or(0)));
    let mut chain: Vec<_> = walk(src, newest).collect();
    chain.reverse();
    let mut t = Tally::new(src, filter);
    let mut raw = Vec::new();
    let (mut adds, mut dels) = (Vec::new(), Vec::new());
    for (_, files) in src.rows_for(chain.clone()) {
        let (a, d) = (t.added, t.removed);
        t.apply(&files);
        adds.push(t.added - a);
        dels.push(t.removed - d);
        raw.push(t.net.clone());
    }
    let fold = Fold::new(&t.names, &t.net);
    let lines = raw
        .iter()
        .map(|r| {
            let mut r = r.clone();
            r.resize(t.names.len(), 0);
            fold.apply(&r)
        })
        .collect();
    Reference {
        ids: chain
            .iter()
            .map(|r| src.commit_id(*r).map(|s| s.to_string()).unwrap_or_default())
            .collect(),
        times: chain.iter().map(|r| src.commit(*r).map_or(0, |c| c.time)).collect(),
        lines,
        adds,
        dels,
    }
}

fn filter() -> PathFilter {
    PathFilter::new(&Options::default(), None)
}

/// Every ranged view reads the same lines, ids, times and range sums as the full chain:
/// a range inside one mark gap, across gaps, at both ends and the whole history.
#[test]
fn views_match_the_full_chain() {
    let recs = history(1500);
    let src = MemorySource::new(&recs);
    let f = filter();
    let want = reference(&src, &f);
    let sk = Arc::new(Skeleton::build(&src, &f).0);
    let n = sk.len();
    assert_eq!(n, 1500);
    assert!(sk.gap < n, "the fixture spans several mark gaps");
    let ranges = [
        None,
        Some((70, 90)),
        Some((10, 1200)),
        Some((0, 5)),
        Some((n - 3, n - 1)),
        Some((0, n - 1)),
        Some((63, 64)),
    ];
    for range in ranges {
        let view = View {
            time: false,
            share: false,
            range,
            picked: range.is_some(),
            period: gitterrace_index::Period::All,
            dflt: gitterrace_index::Period::All,
            held: Held::Commits,
        };
        let ch = sk.view(&src, &f, view);
        for i in wanted(n, range) {
            assert_eq!(ch.lines(i), want.lines[i], "lines at {i} for {range:?}");
            assert_eq!(ch.id(i), want.ids[i], "id at {i} for {range:?}");
            assert_eq!(ch.time(i), want.times[i], "time at {i} for {range:?}");
        }
        let (a, b) = range.unwrap_or((0, n - 1));
        assert_eq!(ch.added, want.adds[a..=b].iter().sum::<u64>(), "{range:?}");
        assert_eq!(ch.removed, want.dels[a..=b].iter().sum::<u64>(), "{range:?}");
    }
}

/// What a skeleton holds is flat in history length: the marks are capped and the kept
/// positions follow the page, so 100 times the history may not grow it more than a little.
#[test]
fn skeleton_size_is_flat_in_history_length() {
    let f = filter();
    let small = history(2_000);
    let large = history(200_000);
    let a = Skeleton::build(&MemorySource::new(&small), &f).0.bytes();
    let b = Skeleton::build(&MemorySource::new(&large), &f).0.bytes();
    assert!(b < 512 * 1024, "skeleton holds {b} bytes at 200k commits");
    assert!(b < a * 4, "skeleton grew from {a} to {b} bytes");
}

/// A tree with signatures, prebuilt binaries and files without an extension next to real
/// languages: the legend names only languages, and every line still counts, under `Other`.
#[test]
fn formats_count_under_other_and_leave_the_legend() {
    let names: Vec<String> = ["Rust", ".asc", "Python", ".prebuilt", "(no extension)", ".png"]
        .map(str::to_owned)
        .to_vec();
    let last = [500, 40, 300, 70, 20, 9];
    let f = Fold::new(&names, &last);
    assert_eq!(f.langs, ["Rust", "Python", "(no extension)", "Other"]);
    let row = f.apply(&last);
    assert_eq!(row, [500, 300, 20, 40 + 70 + 9]);
    assert_eq!(row.iter().sum::<i64>(), last.iter().sum::<i64>(), "no line is lost");
    // Languages alone fold nothing.
    let f = Fold::new(&names[..1], &last[..1]);
    assert_eq!(f.langs, ["Rust"]);
}

/// A history with side branches: every fifth commit on the first-parent line merges a side
/// commit made just before it, so the walk must skip ranks that are not on the chain.
fn merged_history(n: usize) -> Vec<CommitRecord> {
    let mut recs = history(n);
    for i in (5..n).step_by(5) {
        recs[i].parents = vec![format!("{:040x}", i - 2), format!("{:040x}", i - 1)];
    }
    recs
}

/// A source that counts oid lookups and record reads, forwarding everything.
struct Reads<S> {
    pub(super) inner: S,
    lookups: AtomicUsize,
    records: AtomicUsize,
    /// Path ids named through `path`, in call order.
    pub(super) paths: std::sync::Mutex<Vec<u32>>,
}

impl<S> Reads<S> {
    pub(super) fn new(inner: S) -> Self {
        Self {
            inner,
            lookups: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
            paths: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn take(&self) -> (usize, usize) {
        (self.lookups.swap(0, Ordering::Relaxed), self.records.swap(0, Ordering::Relaxed))
    }
}

impl<S: InsightsSource + ?Sized> InsightsSource for Reads<&S> {
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn commit(&self, r: Rank) -> Option<feed::CommitRow> {
        self.records.fetch_add(1, Ordering::Relaxed);
        self.inner.commit(r)
    }
    fn commits_in(&self, r: feed::CommitRange) -> feed::Stream<'_, feed::CommitRow> {
        self.inner.commits_in(r)
    }
    fn rows_for(&self, c: Vec<Rank>) -> feed::Stream<'_, (Rank, Vec<feed::FileRow>)> {
        self.inner.rows_for(c)
    }
    fn commit_id(&self, r: Rank) -> Option<Arc<str>> {
        self.records.fetch_add(1, Ordering::Relaxed);
        self.inner.commit_id(r)
    }
    fn parent_ids(&self, r: Rank) -> Vec<Arc<str>> {
        self.records.fetch_add(1, Ordering::Relaxed);
        self.inner.parent_ids(r)
    }
    fn find_commit(&self, id: &str) -> Result<Option<Rank>, feed::SourceError> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        self.inner.find_commit(id)
    }
    fn agg_author_day(&self, d: std::ops::Range<i64>) -> feed::Stream<'_, feed::AuthorDayRow> {
        self.inner.agg_author_day(d)
    }
    fn agg_path_author(&self, p: &str) -> feed::Stream<'_, feed::PathAuthorRow> {
        self.inner.agg_path_author(p)
    }
    fn postings(&self, k: feed::Posting) -> feed::Stream<'_, feed::Hit> {
        self.inner.postings(k)
    }
    fn rename_edges(&self) -> feed::Stream<'_, feed::RenameEdge> {
        self.inner.rename_edges()
    }
    fn path(&self, id: feed::PathId) -> Option<Arc<str>> {
        if let Ok(mut p) = self.paths.lock() {
            p.push(id.0);
        }
        self.inner.path(id)
    }
    fn find_path(&self, p: &str) -> Result<Option<feed::PathId>, feed::SourceError> {
        self.inner.find_path(p)
    }
    fn paths(&self) -> usize {
        self.inner.paths()
    }
    fn ident(&self, id: feed::IdentId) -> Option<(Arc<str>, Arc<str>)> {
        self.inner.ident(id)
    }
    fn idents(&self) -> usize {
        self.inner.idents()
    }
    fn label(&self, id: feed::LabelId) -> Option<Arc<str>> {
        self.inner.label(id)
    }
    fn find_label(&self, n: &str) -> Result<Option<feed::LabelId>, feed::SourceError> {
        self.inner.find_label(n)
    }
    fn failure(&self) -> Option<feed::SourceError> {
        self.inner.failure()
    }
}

/// The chain by oid lookups, one per step: the walk the page must agree with.
fn chain_by_lookup(src: &dyn InsightsSource) -> Vec<Rank> {
    let newest = src.len().checked_sub(1).map(|i| Rank(u32::try_from(i).unwrap_or(0)));
    std::iter::successors(newest, |r| src.parent_ids(*r).first().and_then(|p| src.commit_of(p)))
        .collect()
}

/// Walking the first-parent chain does no oid index lookup, and reads each record a bounded
/// number of times, whatever the history length. An index lookup per step reads a random
/// block of the oid index: most of a minute of the cold page on a large history. A walk
/// that searched from the top for each parent would read the history once per step and
/// fails the record bound. The chain is the one the lookups give, merges skipped.
#[test]
fn the_chain_walk_reads_each_record_a_bounded_number_of_times() {
    for n in [300, 3000] {
        let recs = merged_history(n);
        let mem = MemorySource::new(&recs);
        let src = Reads::new(&mem);
        let want = chain_by_lookup(src.inner);
        assert!(want.len() < n, "the fixture has side commits");
        src.take();
        let newest = Some(Rank(u32::try_from(n - 1).unwrap_or(0)));
        let got: Vec<Rank> = walk(&src, newest).collect();
        let (lookups, records) = src.take();
        assert_eq!(got, want, "the chain at {n} commits");
        assert_eq!(lookups, 0, "oid lookups walking {n} commits");
        assert!(records <= 3 * n, "{records} record reads walking {n} commits");
        // The whole skeleton too: its walk, its marks and its kept positions.
        let sk = Skeleton::build(&src, &filter()).0;
        let (lookups, records) = src.take();
        assert_eq!(sk.len(), want.len());
        assert_eq!(lookups, 0, "oid lookups building over {n} commits");
        assert!(records <= 4 * n, "{records} record reads building over {n} commits");
    }
}

/// With side branches, every view still reads what the full chain gives.
#[test]
fn views_match_the_full_chain_across_merges() {
    let recs = merged_history(1500);
    let src = MemorySource::new(&recs);
    let f = filter();
    let want = reference(&src, &f);
    let sk = Arc::new(Skeleton::build(&src, &f).0);
    let n = sk.len();
    assert_eq!(n, chain_by_lookup(&src).len());
    for range in [None, Some((70, 90)), Some((10, n - 200)), Some((0, n - 1))] {
        let view = View {
            time: false,
            share: false,
            range,
            picked: range.is_some(),
            period: gitterrace_index::Period::All,
            dflt: gitterrace_index::Period::All,
            held: Held::Commits,
        };
        let ch = sk.view(&src, &f, view);
        for i in wanted(n, range) {
            assert_eq!(ch.lines(i), want.lines[i], "lines at {i} for {range:?}");
            assert_eq!(ch.id(i), want.ids[i], "id at {i} for {range:?}");
        }
    }
}

/// The two skeletons hold the same marks, positions and languages.
fn same(a: &Skeleton, b: &Skeleton, what: &str) {
    assert_eq!(a.n, b.n, "{what}: length");
    assert_eq!(a.gap, b.gap, "{what}: gap");
    assert_eq!(a.langs, b.langs, "{what}: languages");
    assert_eq!(a.marks, b.marks, "{what}: marks");
    assert_eq!(a.fixed, b.fixed, "{what}: positions");
}

/// Guard class: for every warmer that extends a saved state, extending the state of tip A to
/// tip B equals a build from nothing at B. Run through a saved and reloaded state, over a
/// short extension, one that crosses a mark gap, none at all, and the chain growing past the
/// size where the gap doubles. Red when the restored tally, the kept checkpoints or the
/// positions below the old tip differ from a fresh pass.
#[test]
fn extending_a_saved_state_equals_a_fresh_build() {
    let f = filter();
    for (a, b) in [(1, 2), (10, 40), (500, 1500), (1500, 1500), (131_000, 131_300)] {
        let recs = history(b);
        let at_b = MemorySource::new(&recs);
        let at_a = MemorySource::new(&recs[..a]);
        let (_, saved) = Skeleton::build(&at_a, &f);
        let saved = State::decode(&saved.encode()).expect("a saved state reads back");
        let (extended, state) =
            Skeleton::extend(&at_b, &f, &saved).expect("A is on the chain of B");
        let (fresh, fresh_state) = Skeleton::build(&at_b, &f);
        same(&extended, &fresh, &format!("{a} to {b}"));
        assert_eq!(state, fresh_state, "{a} to {b}: the state to extend next");
    }
}

/// Guard class: a state that does not fit is refused, never used. A tip that is not on the
/// chain (another branch, a rewritten history) and a gap that is no divisor of the new one
/// both give `None`, so the caller builds from nothing.
#[test]
fn a_state_off_this_chain_is_refused() {
    let f = filter();
    let recs = history(300);
    let src = MemorySource::new(&recs);
    let (_, mut state) = Skeleton::build(&MemorySource::new(&recs[..100]), &f);
    state.tip = "f".repeat(40);
    assert!(Skeleton::extend(&src, &f, &state).is_none(), "a foreign tip");
    let (_, mut state) = Skeleton::build(&MemorySource::new(&recs[..100]), &f);
    state.gap = 96;
    assert!(Skeleton::extend(&src, &f, &state).is_none(), "a gap that does not divide");
}

/// Guard class: the reader of a state file bounds itself by the file. Any prefix or flipped
/// byte of a good state reads as none or as a state that still adds up, and a count past what
/// the file holds is refused before it is allocated.
#[test]
fn a_damaged_state_never_decodes_into_nonsense() {
    let recs = history(300);
    let (_, state) = Skeleton::build(&MemorySource::new(&recs), &filter());
    let bytes = state.encode();
    assert_eq!(State::decode(&bytes).as_ref(), Some(&state));
    for cut in 0..bytes.len() {
        assert!(State::decode(&bytes[..cut]).is_none(), "cut at {cut}");
    }
    let mut huge = bytes;
    huge[..8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(State::decode(&huge).is_none(), "an absurd length");
}

/// Guard class: the key follows everything the totals depend on. Another rule set, another
/// ignore file or other ignore text is another key, and the order of the pairs is kept.
#[test]
fn the_state_key_follows_the_rules_and_the_ignore_files() {
    let ig = |p: &str, t: &str| vec![(p.to_owned(), t.to_owned())];
    let k = |rules: &str, i: &[(String, String)]| super::state::key(rules, i);
    let base = k("r", &ig(".gitignore", "*.o\n"));
    assert_eq!(base, k("r", &ig(".gitignore", "*.o\n")));
    assert_ne!(base, k("s", &ig(".gitignore", "*.o\n")));
    assert_ne!(base, k("r", &ig(".gitignore", "*.a\n")));
    assert_ne!(base, k("r", &ig("d/.gitignore", "*.o\n")));
    assert_ne!(k("r", &ig("ab", "c")), k("r", &ig("a", "bc")), "no run-together");
}

/// Guard class: a typo in the extension list. Every entry is a dot extension with no stray
/// characters, and the lookup finds exactly the entries.
#[test]
fn every_format_extension_is_a_dotted_word() {
    for e in super::NOT_LANGUAGES.split(' ') {
        assert!(e.len() > 1 && e.starts_with('.') && !e[1..].contains(['.', ' ']), "{e:?}");
        assert!(!super::is_language(e), "{e}");
    }
    assert!(super::is_language(".rs") && super::is_language(""));
}
