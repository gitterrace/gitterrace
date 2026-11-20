//! The same query string gives an equal `Scope` whichever way it is read: as `ScopeParams`
//! directly (the form web pages convert to) and through the API's own `Params`. A table of
//! strings covers defaults, empties and junk. The web `InsightsQuery` joins this table when it
//! builds its scope through `ScopeParams`.
#![expect(clippy::expect_used, reason = "test setup")]

use axum::extract::Query;
use axum::http::Uri;
use gitterrace_api::Params;
use gitterrace_index::{Options, Period, Scope, ScopeError, ScopeParams};

const TABLE: &[&str] = &[
    "",
    "period=",
    "period=1w",
    "period=all&bots=1",
    "period=3m&q=author:ann@x.io%20path:/src/%20type:fix%20scope:web",
    "q=%20&period=",
    "bots=yes&ws=1&generated=0&ignore=0&exclude=*.md,%20,docs/**",
    "period=1y&from=2026-01-01&to=2026-03-31",
    "from=2026-01-01",
    "unknown=1&limit=5&label=scope",
];

/// Strings both readers must refuse, with the same error.
const BAD: &[&str] = &["period=2w", "from=2026-02-30", "from=2026-03-02&to=2026-03-01"];

fn uri(q: &str) -> Uri {
    format!("/x?{q}").parse().expect("uri")
}

fn direct(q: &str) -> Result<Scope, ScopeError> {
    let Query(p): Query<ScopeParams> = Query::try_from_uri(&uri(q)).expect("parse");
    Scope::from_params(&p, Options::default(), Period::All)
}

fn via_api(q: &str) -> Result<Scope, ScopeError> {
    let Query(p): Query<Params> = Query::try_from_uri(&uri(q)).expect("parse");
    let Query(range): Query<ScopeParams> = Query::try_from_uri(&uri(q)).expect("parse");
    let sp = ScopeParams {
        period: p.period,
        from: range.from,
        to: range.to,
        select: p.select,
        options: p.options,
    };
    Scope::from_params(&sp, Options::default(), Period::All)
}

#[test]
fn every_reader_gives_an_equal_scope() {
    for q in TABLE {
        let a = direct(q).unwrap_or_else(|e| panic!("{q}: {e}"));
        assert_eq!(Ok(a), via_api(q), "{q}");
    }
    for q in BAD {
        assert!(direct(q).is_err(), "{q} accepted");
        assert_eq!(direct(q), via_api(q), "{q}");
    }
}

/// `type`, `scope`, `author` and `path` are no parameters: they select nothing. The API refuses
/// them before this reader runs; here they read as unset.
#[test]
fn one_key_forms_select_nothing() {
    let a = direct("type=fix&scope=web&author=ann@x.io&path=src").expect("ok");
    assert!(a.matcher().is_empty());
}

#[test]
fn empties_mean_unset() {
    let a = direct("period=&q=%20").expect("ok");
    assert_eq!(a, direct("").expect("ok"));
    assert!(a.matcher().is_empty());
}

#[test]
fn merges_param_reads_the_same_everywhere_and_is_off_by_default() {
    assert!(!direct("").expect("ok").options().include_merges);
    for (q, on) in [("merges=1", true), ("merges=0", false), ("merges=junk", false)] {
        let a = direct(q).expect("ok");
        assert_eq!(a.options().include_merges, on, "{q}");
        assert_eq!(Ok(a), via_api(q), "{q}");
    }
}
