use proptest::prelude::*;

use super::{KEYWORDS, Leaf, Query, Term, When};
use gitterrace_types::Period;

fn q(s: &str) -> Query {
    Query::parse(s).unwrap_or_else(|e| panic!("{s:?}: {e}"))
}

#[test]
fn every_keyword_example_parses() {
    for k in KEYWORDS {
        if let Some(ex) = k.example {
            let parsed = q(ex);
            assert!(!parsed.is_empty(), "{ex}");
            assert_eq!(q(&parsed.to_string()), parsed, "{ex}");
        }
    }
}

#[test]
fn reads_terms_and_writes_them_canonically() {
    let got = q("crash !merge   author:\"Ann Lee\",bob@x.io type:fix,feat path:/src/ after:3m");
    assert_eq!(
        got.to_string(),
        "crash !merge author:\"Ann Lee\",bob@x.io type:fix,feat path:src after:3m"
    );
    assert_eq!(q("period:all"), Query::default());
    assert_eq!(q("period:quarter").to_string(), "after:3m");
    assert_eq!(q("before:2026-01-31").to_string(), "before:2026-01-31");
    assert_eq!(q("\"merge\"").terms()[0].leaf, Leaf::Text("merge".into()));
    assert_eq!(q("  ").to_string(), "");
}

#[test]
fn refusals_say_what_is_supported() {
    for (s, at) in [("a | b", 2), ("(a)", 0), ("!(a b)", 0), ("fix)", 3)] {
        let e = Query::parse(s).expect_err(s);
        assert!(e.message.contains("Supported:"), "{s}: {e}");
        assert_eq!(e.start, at, "{s}");
    }
    for s in [
        "author:",
        "type:fix,",
        "after:soon",
        "after:all",
        "\"open",
        "!",
        "path:/",
        "nope:x",
        "committer:ann",
    ] {
        let e = Query::parse(s).expect_err(s);
        assert!(e.end > e.start && e.end <= s.len().max(1), "{s}: {e:?}");
    }
    assert!(
        Query::parse("committer:ann")
            .expect_err("committer")
            .message
            .contains("not supported")
    );
}

#[test]
fn chips_remove_one_term() {
    let a = q("type:fix author:ann crash");
    assert_eq!(a.without(1).to_string(), "type:fix crash");
    let b = a.replace(
        |l| matches!(l, Leaf::After(_)),
        Some(Term {
            neg: false,
            leaf: Leaf::After(When::Period(Period::Week)),
        }),
    );
    assert_eq!(b.to_string(), "type:fix author:ann crash after:1w");
    let c = q("after:3m fix").replace(|l| matches!(l, Leaf::After(_)), b.terms()[3].clone().into());
    assert_eq!(c.to_string(), "after:1w fix");
    assert!(Query::parse(&"a ".repeat(2_000)).is_err());
}

fn value() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 _.@:,!\"\\\\|()/-]{1,8}"
}

fn path() -> impl Strategy<Value = String> {
    "[a-z0-9 ,.!\"_-]{1,4}(/[a-z0-9 ._-]{1,4}){0,2}"
}

fn when() -> impl Strategy<Value = When> {
    prop_oneof![
        prop::sample::select(vec![
            Period::Week,
            Period::Month,
            Period::Quarter,
            Period::Year
        ])
        .prop_map(When::Period),
        (1970..2100i32, 1..=12u8, 1..=28u8).prop_map(|(y, m, d)| When::Day { y, m, d }),
    ]
}

fn leaf() -> impl Strategy<Value = Leaf> {
    let list = || prop::collection::vec(value(), 1..3);
    prop_oneof![
        list().prop_map(Leaf::Kind),
        list().prop_map(Leaf::Scope),
        list().prop_map(Leaf::Author),
        prop::collection::vec(path(), 1..3).prop_map(Leaf::Path),
        when().prop_map(Leaf::After),
        when().prop_map(Leaf::Before),
        Just(Leaf::Merge),
        value().prop_map(Leaf::Text),
    ]
}

proptest! {
    #[test]
    fn serialize_then_parse_is_identity(
        terms in prop::collection::vec((any::<bool>(), leaf()), 0..6)
    ) {
        let query = Query::new(terms.into_iter().map(|(neg, leaf)| Term { neg, leaf }).collect());
        let text = query.to_string();
        prop_assert_eq!(Query::parse(&text), Ok(query), "{}", text);
    }

    #[test]
    fn parse_never_panics_and_canonical_text_is_a_fixed_point(s in "[a-z:!\"\\\\ ,()|/0-9-]{0,24}") {
        if let Ok(once) = Query::parse(&s) {
            let text = once.to_string();
            prop_assert_eq!(Query::parse(&text).map(|q| q.to_string()), Ok(text));
        }
    }
}
