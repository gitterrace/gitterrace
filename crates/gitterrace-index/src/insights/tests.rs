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
