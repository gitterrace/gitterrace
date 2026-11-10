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
fn parses_and_ignores_empty() {
    let f = parse("type=fix&scope=&path=crates%2Fweb&bogus=1");
    assert_eq!(
        (f.kind(), f.scope(), f.path()),
        (Some("fix"), None, Some("crates/web"))
    );
}

/// Every older link form (`?type= ?scope= ?author= ?path= ?period=`) still selects the
/// commits its `?q=` form selects, and writes links in that form.
#[test]
fn old_link_forms_select_what_q_selects() {
    // (old link, its `q` form, the canonical text both read as)
    for (old, q, text) in [
        ("type=fix", "type:fix", "type:fix"),
        ("scope=none", "scope:none", "scope:none"),
        ("author=ann", "author:ann", "author:ann"),
        (
            "path=%2Fcrates%2Fweb%2F",
            "path:crates/web",
            "path:crates/web",
        ),
        ("period=3m", "period:3m", "after:3m"),
        ("period=all", "period:all", ""),
        (
            "type=fix&author=ann&period=1y",
            "type:fix author:ann after:1y",
            "type:fix author:ann after:1y",
        ),
    ] {
        let (a, b) = (parse(old), parse(&format!("q={}", crate::fmt::enc_seg(q))));
        assert_eq!(a.query_terms().to_string(), text, "{old}");
        assert_eq!(a, b, "{old}");
        assert_eq!(ids(&a), ids(&b), "{old}");
    }
    assert_eq!(parse("period=3m").period_value(), Some(Period::Quarter));
    // The whole-repository row links an empty path: that removes the term, as before.
    assert_eq!(
        parse("path=src&type=fix").link("/s", Dim::Path, ""),
        "/s?q=type%3Afix"
    );
    assert_eq!(
        parse("author=a").link("/s", Dim::Path, "/"),
        "/s?q=author%3Aa"
    );
    // `all` overrides a configured default period, so it survives into every link.
    assert_eq!(parse("period=all").period_value(), Some(Period::All));
    assert_eq!(
        parse("period=all&type=fix").query(&[]),
        "?q=type%3Afix&period=all"
    );
}

#[test]
fn query_round_trips_and_encodes() {
    let f = parse("type=fix&author=Ann%20B&period=3m");
    assert_eq!(
        f.query(&[]),
        "?q=type%3Afix%20author%3A%22Ann%20B%22%20after%3A3m"
    );
    assert_eq!(parse(&f.query(&[])[1..]), f);
    assert_eq!(Filter::default().query(&[]), "");
    assert_eq!(
        f.with(Dim::Kind, None).query(&[("page", "2")]),
        "?q=author%3A%22Ann%20B%22%20after%3A3m&page=2"
    );
}

#[test]
fn a_refused_query_keeps_its_text_and_the_rest() {
    let f = parse("q=a%20%7C%20b&type=fix");
    assert_eq!(f.text(), "a | b");
    assert!(f.error().is_some_and(|e| e.message.contains("Supported")));
    assert_eq!(f.kind(), Some("fix"));
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
    let out = recs_of(&kept(&parse("path=crates/web"), &sample()));
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
    for q in ["period=1w", "period=all&type=fix", "period=1y&path=src"] {
        let bar = parse(q).bar("/stats", &MemorySource::new(&[]), Period::Week);
        assert!(bar.chips.iter().all(|c| c.dim != "after"), "{q}");
    }
}

#[test]
fn bar_chips_remove_one_filter() {
    let bar =
        parse("type=fix&scope=web").bar("/stats/types", &MemorySource::new(&[]), Period::Month);
    assert_eq!(bar.chips.len(), 2);
    assert_eq!(bar.chips[0].href, "/stats/types?q=scope%3Aweb");
    assert_eq!(bar.chips[1].href, "/stats/types?q=type%3Afix");
    // With no period asked for, the one in force is marked: the select never shows another.
    assert_eq!(bar.period.choices.iter().filter(|p| p.on).count(), 1);
    let neg = Filter::parse("!path:docs").chips("/log/HEAD", None);
    assert_eq!((neg[0].dim, neg[0].label.as_str()), ("path", "not docs"));
}
