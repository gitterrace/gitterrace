//! Unit tests for the insights entry points.

use super::{Insights, Options, bus_factor, fixtures};

#[test]
fn derived_tables_are_built_once_per_key_and_options() {
    let recs = fixtures::chain();
    let src = crate::feed::MemorySource::new(&recs);
    let copy = crate::feed::MemorySource::new(&recs);
    let built = std::cell::Cell::new(0);
    let ignore = || {
        built.set(built.get() + 1);
        None
    };
    let store = super::SharedTables::new();
    let shared = |s, o, k| Insights::shared_source(&store, s, o, k, ignore);
    let a = shared(&src, Options::default(), "t1\u{0}gen1");
    let b = shared(&src, Options::default(), "t1\u{0}gen1");
    assert!(a.shares_tables_with(&b), "second page rebuilt the tables");
    assert_eq!(built.get(), 1);
    let gen2 = shared(&src, Options::default(), "t1\u{0}gen2");
    let bots = Options { include_bots: true, ..Options::default() };
    let other = shared(&src, bots, "t1\u{0}gen1");
    let moved = shared(&copy, Options::default(), "t1\u{0}gen1");
    for x in [&gen2, &other, &moved] {
        assert!(!a.shares_tables_with(x), "stale tables reused");
    }
    assert_eq!(a.hotspots(&[], super::Period::All).files.len(), 0);
}

#[test]
fn bus_factor_counts_who_hold_at_least_half() {
    // The 50/50 split is the case where the surfaces disagreed.
    assert_eq!(bus_factor([5.0, 5.0]), 1);
    assert_eq!(bus_factor([6.0, 3.0, 1.0]), 1);
    assert_eq!(bus_factor([3.0, 3.0, 3.0, 1.0]), 2);
    assert_eq!(bus_factor([1.0, 4.0]), 1);
    assert_eq!(bus_factor([0.0, 0.0]), 0);
    assert_eq!(bus_factor(std::iter::empty()), 0);
}

mod decide {
    use super::super::decide_with;
    use std::sync::Arc;

    fn paths(n: usize) -> Vec<Arc<str>> {
        (0..n).map(|i| Arc::from(format!("f{i}"))).collect()
    }

    #[test]
    fn decisions_keep_the_order_and_length_of_the_paths() {
        let p = paths(1000);
        let got = decide_with(&p, |s| s.ends_with('7'));
        assert_eq!(got.len(), p.len());
        assert!(got.iter().zip(&p).all(|(g, s)| *g == s.ends_with('7')));
    }

    #[test]
    #[should_panic(expected = "filter boom")]
    fn a_panicking_filter_panics_the_call_instead_of_shortening_the_table() {
        let p = paths(1000);
        let _ = decide_with(&p, |s| {
            assert!(s != "f500", "filter boom");
            false
        });
    }
}

/// A source whose first full commit pass fails: it streams nothing and reports the failure, as
/// a block that fails its crc does, and every later call reads whole. Which read fails is the
/// only thing a run of the web test leaves to chance, so this fixes it.
struct FailsFirstPass<'a> {
    inner: crate::feed::MemorySource<'a>,
    failed: std::sync::atomic::AtomicBool,
}

impl crate::feed::InsightsSource for FailsFirstPass<'_> {
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn commit(&self, r: crate::feed::Rank) -> Option<crate::feed::CommitRow> {
        self.inner.commit(r)
    }
    fn commits_in(
        &self,
        r: crate::feed::CommitRange,
    ) -> crate::feed::Stream<'_, crate::feed::CommitRow> {
        if !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Box::new(std::iter::empty());
        }
        self.inner.commits_in(r)
    }
    fn rows_for(
        &self,
        c: Vec<crate::feed::Rank>,
    ) -> crate::feed::Stream<'_, (crate::feed::Rank, Vec<crate::feed::FileRow>)> {
        self.inner.rows_for(c)
    }
    fn commit_id(&self, r: crate::feed::Rank) -> Option<std::sync::Arc<str>> {
        self.inner.commit_id(r)
    }
    fn parent_ids(&self, r: crate::feed::Rank) -> Vec<std::sync::Arc<str>> {
        self.inner.parent_ids(r)
    }
    fn find_commit(&self, id: &str) -> Result<Option<crate::feed::Rank>, crate::feed::SourceError> {
        self.inner.find_commit(id)
    }
    fn agg_author_day(
        &self,
        d: std::ops::Range<i64>,
    ) -> crate::feed::Stream<'_, crate::feed::AuthorDayRow> {
        self.inner.agg_author_day(d)
    }
    fn agg_path_author(&self, p: &str) -> crate::feed::Stream<'_, crate::feed::PathAuthorRow> {
        self.inner.agg_path_author(p)
    }
    fn postings(&self, k: crate::feed::Posting) -> crate::feed::Stream<'_, crate::feed::Hit> {
        self.inner.postings(k)
    }
    fn rename_edges(&self) -> crate::feed::Stream<'_, crate::feed::RenameEdge> {
        self.inner.rename_edges()
    }
    fn path(&self, id: crate::feed::PathId) -> Option<std::sync::Arc<str>> {
        self.inner.path(id)
    }
    fn find_path(&self, p: &str) -> Result<Option<crate::feed::PathId>, crate::feed::SourceError> {
        self.inner.find_path(p)
    }
    fn paths(&self) -> usize {
        self.inner.paths()
    }
    fn ident(
        &self,
        id: crate::feed::IdentId,
    ) -> Option<(std::sync::Arc<str>, std::sync::Arc<str>)> {
        self.inner.ident(id)
    }
    fn idents(&self) -> usize {
        self.inner.idents()
    }
    fn label(&self, id: crate::feed::LabelId) -> Option<std::sync::Arc<str>> {
        self.inner.label(id)
    }
    fn find_label(
        &self,
        n: &str,
    ) -> Result<Option<crate::feed::LabelId>, crate::feed::SourceError> {
        self.inner.find_label(n)
    }
    fn failure(&self) -> Option<crate::feed::SourceError> {
        self.failed
            .load(std::sync::atomic::Ordering::SeqCst)
            .then(|| crate::feed::SourceError("block crc".into()))
    }
}

/// Guard class: tables built while a read failed have a short people table, and no page
/// reads them. The read that fails may be any of the ones the build makes, so the first pass
/// is the one that fails here, every run. Red before: the next read of commits found an author
/// the empty table had no row for and indexed past it (a panic), and the shared store kept
/// the short tables for the next page over the same key. The second defect: a failed build
/// that is cached, which serves the next page with data from the failed read.
#[test]
fn tables_built_over_a_failed_read_are_never_indexed_or_kept() {
    let recs = fixtures::chain();
    let src = FailsFirstPass {
        inner: crate::feed::MemorySource::new(&recs),
        failed: std::sync::atomic::AtomicBool::new(false),
    };
    let store = super::SharedTables::new();
    let ins = Insights::shared_source(&store, &src, Options::default(), "t\u{0}1", || None);
    assert!(store.is_empty(), "tables built over a failed read were kept");
    let _ = ins.every_totals();
    let row = crate::feed::InsightsSource::commits_in(&src, crate::feed::CommitRange::All)
        .next()
        .expect("a commit");
    assert_eq!(ins.person_of(row.ident), None);
    assert!(!ins.counted_row(&row));
    assert!(crate::feed::InsightsSource::failure(&src).is_some());
}
