use std::sync::Arc;

use gitterrace_index::feed::MemorySource;
use gitterrace_index::{CommitRecord, FileStat};

use super::{Dim, Filter, Period, under};
use crate::insights::Src;

fn parse(q: &str) -> Filter {
    let uri: axum::http::Uri = format!("/?{q}").parse().expect("uri");
    axum::extract::Query::try_from_uri(&uri).expect("parse").0
}

fn rec(
    id: &str,
    who: &str,
    day: i64,
    kind: Option<&str>,
    scope: Option<&str>,
    path: &str,
) -> CommitRecord {
    CommitRecord {
        id: id.to_owned(),
        author: who.to_owned(),
        time: day * 86_400,
        kind: kind.map(str::to_owned),
        scope: scope.map(str::to_owned),
        files: vec![
            FileStat {
                path: path.into(),
                additions: 3,
                ..FileStat::default()
            },
            FileStat {
                path: "other/x.rs".into(),
                additions: 1,
                ..FileStat::default()
            },
        ],
        ..CommitRecord::default()
    }
}

/// The commits a filter's selection keeps, as a scoped source.
fn kept(f: &Filter, all: &Src) -> Src {
    let m = gitterrace_index::query::Matcher::new(f.query_terms());
    Arc::new(gitterrace_index::Scoped::new(all, &m, None))
}

fn sample() -> Src {
    Arc::new(MemorySource::shared(Arc::new(sample_records())))
}

/// What a filtered source holds: each commit id with its kept rows' paths.
fn recs_of(s: &Src) -> Vec<CommitRecord> {
    use gitterrace_index::feed::CommitRange;
    let ranks: Vec<_> = s.commits_in(CommitRange::All).map(|c| c.rank).collect();
    s.rows_for(ranks)
        .map(|(rank, rows)| CommitRecord {
            id: s.commit_id(rank).as_deref().unwrap_or_default().to_owned(),
            files: rows
                .iter()
                .map(|r| gitterrace_index::FileStat {
                    path: s.path(r.path).unwrap_or_default(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .collect()
}

fn sample_records() -> Vec<CommitRecord> {
    vec![
        rec("a", "ann", 1, Some("fix"), Some("web"), "crates/web/a.rs"),
        rec("b", "bob", 50, Some("feat"), None, "crates/git/b.rs"),
        rec("c", "ann", 99, None, None, "crates/web/c.rs"),
        rec(
            "d",
            "ann",
            100,
            Some("fix"),
            Some("git"),
            "crates/webby/d.rs",
        ),
    ]
}

fn ids(f: &Filter) -> Vec<String> {
    recs_of(&kept(f, &sample()))
        .iter()
        .map(|r| r.id.clone())
        .collect()
}

#[test]
fn parses_q_and_ignores_empty() {
    let f = parse("q=type%3Afix%20path%3Acrates%2Fweb&bogus=1");
    assert_eq!(
        (f.kind(), f.scope(), f.path()),
        (Some("fix"), None, Some("crates/web"))
    );
}

/// The one-key forms (`?type= ?scope= ?author= ?path=`) select nothing: only `?q=` does, so a
/// link keeps every commit and writes no term for them. Red while they were read as terms.
#[test]
fn one_key_forms_no_longer_filter() {
    let all = ids(&Filter::default());
    for old in [
        "type=fix",
        "scope=none",
        "author=ann",
        "path=%2Fcrates%2Fweb%2F",
        "type=fix&author=ann",
    ] {
        let f = parse(old);
        assert_eq!(f.query_terms().to_string(), "", "{old}");
        assert_eq!(ids(&f), all, "{old}");
        assert_eq!(f.query(&[]), "", "{old}");
    }
}

/// `?period=` is the period control's parameter: it reads as the window, and `all` survives
/// into every link so it overrides a configured default period.
#[test]
fn the_period_parameter_sets_the_window() {
    assert_eq!(parse("period=3m").query_terms().to_string(), "after:3m");
    assert_eq!(parse("period=3m").period_value(), Some(Period::Quarter));
    assert_eq!(parse("period=all").period_value(), Some(Period::All));
    assert_eq!(
        parse("period=all&q=type%3Afix").query(&[]),
        "?q=type%3Afix&period=all"
    );
    // A period it does not know is refused, as bad text is, never read as no window.
    let bad = parse("period=soon&q=type%3Afix");
    assert!(bad.error().is_some_and(|e| e.message.contains("`period`")));
    // The whole-repository row links an empty path: that removes the term.
    assert_eq!(
        parse("q=path%3Asrc%20type%3Afix").link("/s", Dim::Path, ""),
        "/s?q=type%3Afix"
    );
    assert_eq!(
        parse("q=author%3Aa").link("/s", Dim::Path, "/"),
        "/s?q=author%3Aa"
    );
}

#[test]
fn query_round_trips_and_encodes() {
    let f = parse("q=type%3Afix%20author%3A%22Ann%20B%22&period=3m");
    assert_eq!(
        f.query(&[]),
        "?q=after%3A3m%20type%3Afix%20author%3A%22Ann%20B%22"
    );
    assert_eq!(parse(&f.query(&[])[1..]), f);
    assert_eq!(Filter::default().query(&[]), "");
    assert_eq!(
        f.with(Dim::Kind, None).query(&[("page", "2")]),
        "?q=after%3A3m%20author%3A%22Ann%20B%22&page=2"
    );
}

#[test]
fn a_refused_query_keeps_its_text_and_the_period() {
    let f = parse("q=a%20%7C%20b&period=1m");
    assert_eq!(f.text(), "a | b");
    assert!(f.error().is_some_and(|e| e.message.contains("Supported")));
    assert_eq!(f.period_value(), Some(Period::Month));
}

#[test]
fn filters_stack() {
    let q = |s: &str| Filter::parse(s);
    assert_eq!(ids(&Filter::default()), ["a", "b", "c", "d"]);
    assert_eq!(ids(&q("type:fix")), ["a", "d"]);
    assert_eq!(ids(&q("type:fix scope:web")), ["a"]);
    assert_eq!(ids(&q("type:other")), ["c"]);
    assert_eq!(ids(&q("scope:none")), ["b", "c"]);
    assert_eq!(ids(&q("author:ann type:fix")), ["a", "d"]);
    assert_eq!(ids(&q("type:nothing")), Vec::<String>::new());
    assert_eq!(ids(&q("!author:ann")), ["b"]);
}

#[test]
fn path_filter_keeps_only_files_under_prefix() {
    let out = recs_of(&kept(&parse("q=path%3Acrates%2Fweb"), &sample()));
    assert_eq!(
        out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["a", "c"]
    );
    assert!(out.iter().all(|r| r.files.len() == 1));
    assert!(under("crates/web/a.rs", "crates/web") && !under("crates/webby/d.rs", "crates/web"));
}

#[test]
fn the_bar_never_builds_a_period_chip() {
    // The segmented control shows the period and All resets it: a chip would repeat it and
    // push the control sideways each time a period is chosen.
    for q in [
        "period=1w",
        "period=all&q=type%3Afix",
        "q=after%3A1y%20path%3Asrc",
    ] {
        let bar = parse(q).bar("/stats", &MemorySource::new(&[]), Period::Week);
        assert!(bar.chips.iter().all(|c| c.dim != "after"), "{q}");
    }
}

#[test]
fn bar_chips_remove_one_filter() {
    let bar =
        parse("q=type%3Afix%20scope%3Aweb").bar("/stats", &MemorySource::new(&[]), Period::Month);
    assert_eq!(bar.chips.len(), 2);
    assert_eq!(bar.chips[0].href, "/stats?q=scope%3Aweb");
    assert_eq!(bar.chips[1].href, "/stats?q=type%3Afix");
    // With no period asked for, the one in force is marked: the select never shows another.
    assert_eq!(bar.period.choices.iter().filter(|p| p.on).count(), 1);
    let neg = Filter::parse("!path:docs").chips("/log/HEAD", None);
    assert_eq!((neg[0].dim, neg[0].label.as_str()), ("path", "not docs"));
}
