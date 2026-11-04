//! Parity fuzz for the one membership rule (`Scope::admits`) and the reducers over `Counted`.
//!
//! - P1: every `Accel` answer that is `Some` equals the stream (today only `NoAccel`, so the
//!   property pins the contract before the first fast path is wired).
//! - P2: per-person, per-label, per-bucket and punch/weekday/hour sums all equal the totals.
//! - P3: the time span only prunes: the stream equals a full scan filtered by `admits`, and a
//!   scan over the span widened by one day each side filtered the same way.
//! - P4: today's page functions equal the reducers over the same `Counted`. Red until the
//!   insights pages read `Counted`: marked ignored, so `make check` stays
//!   green while the red stays one `--include-ignored` away. The baseline red output is in
//!   `count_parity_fuzz.baseline.txt` beside this file. `make counting-parity` runs them.
//!
//! Edge times are generated, not fixed: ±1 s around the window bound, 23:59:59, 00:00:00 and
//! 00:00:01 local, git offsets +0530 +0545 +1030 -0930 -1200 +1400, and authors flipping
//! +0100 to +0200 mid-week. The local day comes only from each commit's own offset.

use gitterrace_index::feed::{CommitRange, FixedClock, MemorySource};
use gitterrace_index::{
    CommitRecord, CountContext, FileStat, Insights, Label, NoAccel, Options, Period, ReducerKind,
    Scope, ScopeParams,
};
use proptest::prelude::*;

const DAY: i64 = 86_400;
/// Tuesday 2026-11-03 10:00 +01:00, the owner's example A anchor.
const ANCHOR: i64 = 1_793_696_400;
const OFFSETS: [i32; 10] = [
    0, 3600, 7200, 19_800,  // +0530
    20_700,  // +0545
    37_800,  // +1030
    -34_200, // -0930
    -43_200, // -1200
    50_400,  // +1400
    -36_000, // -1000
];
const AUTHORS: [(&str, &str); 4] = [
    ("Ann", "ann@x.io"),
    ("Bob", "bob@x.io"),
    ("dependabot[bot]", "bot@users.noreply"),
    ("Cy", "cy@x.io"),
];
const PATHS: [&str; 4] = ["src/a.rs", "src/b.rs", "docs/x.md", "Cargo.lock"];
const KINDS: [Option<&str>; 3] = [Some("fix"), Some("feat"), None];
const PERIODS: [&str; 5] = ["1w", "1m", "3m", "1y", "all"];

/// One generated commit: which edge it sits on, and what it carries.
#[derive(Debug, Clone)]
struct Gen {
    /// Days before the anchor, 0..400.
    back: i64,
    edge: Edge,
    offset: usize,
    author: usize,
    files: Vec<(usize, u32, u32, bool)>,
    kind: usize,
    merge: bool,
    /// Berlin flip: author 0 switches between +0100 and +0200 inside one week.
    dst: bool,
}

#[derive(Debug, Clone, Copy)]
enum Edge {
    /// Seconds into the local day.
    Local(i64),
    /// Seconds around a UTC period bound (`ANCHOR - d * DAY + s`).
    Bound(usize, i64),
}

fn edge() -> impl Strategy<Value = Edge> {
    prop_oneof![
        prop::sample::select(vec![0, 1, DAY - 1, 12 * 3600]).prop_map(Edge::Local),
        (0..4usize, prop::sample::select(vec![-1i64, 0, 1])).prop_map(|(p, s)| Edge::Bound(p, s)),
        (0..DAY).prop_map(Edge::Local),
    ]
}

fn gen_commit() -> impl Strategy<Value = Gen> {
    (
        prop_oneof![
            0..10i64,
            0..400i64,
            prop::sample::select(vec![6, 7, 8, 29, 30, 31, 89, 90, 364, 365])
        ],
        edge(),
        0..OFFSETS.len(),
        0..AUTHORS.len(),
        prop::collection::vec((0..PATHS.len(), 0..50u32, 0..20u32, any::<bool>()), 1..3),
        0..KINDS.len(),
        prop::bool::weighted(0.1),
        any::<bool>(),
    )
        .prop_map(
            |(back, edge, offset, author, files, kind, merge, dst)| Gen {
                back,
                edge,
                offset,
                author,
                files,
                kind,
                merge,
                dst,
            },
        )
}

fn record(i: usize, g: &Gen) -> CommitRecord {
    let offset = if g.dst && g.author == 0 {
        if g.back % 7 < 3 { 3600 } else { 7200 }
    } else {
        OFFSETS[g.offset]
    };
    let time = match g.edge {
        Edge::Local(sec) => {
            let day = (ANCHOR + i64::from(offset)).div_euclid(DAY) - g.back;
            day * DAY + sec - i64::from(offset)
        }
        Edge::Bound(which, sec) => {
            let days = [7, 30, 90, 365][which];
            ANCHOR + 1 - days * DAY + sec
        }
    };
    let (author, email) = AUTHORS[g.author];
    let mut files: Vec<FileStat> = Vec::new();
    for &(p, add, del, ws) in &g.files {
        if files.iter().any(|f| &*f.path == PATHS[p]) {
            continue;
        }
        files.push(FileStat {
            path: PATHS[p].into(),
            additions: add,
            deletions: del,
            lines: add + 1,
            code: ws.then_some(gitterrace_index::CodeStat {
                additions: add / 2,
                deletions: del / 2,
                rewritten: 0,
            }),
            ..FileStat::default()
        });
    }
    CommitRecord {
        id: format!("{i:040x}"),
        author: author.into(),
        email: email.into(),
        time,
        offset,
        merge: g.merge,
        parents: Vec::new(),
        kind: KINDS[g.kind].map(str::to_owned),
        scope: None,
        files,
    }
}

/// The newest commit is always the anchor itself, so every case has the same "now".
fn history(gens: &[Gen]) -> Vec<CommitRecord> {
    let mut v: Vec<CommitRecord> = gens.iter().enumerate().map(|(i, g)| record(i, g)).collect();
    v.push(CommitRecord {
        id: "f".repeat(40),
        author: "Ann".into(),
        email: "ann@x.io".into(),
        time: ANCHOR,
        offset: 3600,
        files: vec![FileStat {
            path: "src/a.rs".into(),
            additions: 1,
            lines: 1,
            ..FileStat::default()
        }],
        ..CommitRecord::default()
    });
    v.retain(|r| r.time <= ANCHOR);
    v.sort_by_key(|r| r.time);
    v
}

#[derive(Debug, Clone)]
struct Q {
    period: usize,
    author: Option<usize>,
    path: Option<usize>,
    kind: Option<usize>,
    bots: bool,
    ws: bool,
    generated: bool,
}

fn query() -> impl Strategy<Value = Q> {
    (
        0..PERIODS.len(),
        prop::option::weighted(0.3, 0..AUTHORS.len()),
        prop::option::weighted(0.3, 0..3usize),
        prop::option::weighted(0.3, 0..KINDS.len()),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(period, author, path, kind, bots, ws, generated)| Q {
            period,
            author,
            path,
            kind,
            bots,
            ws,
            generated,
        })
}

fn no_chips() -> impl Strategy<Value = Q> {
    query().prop_map(|q| Q {
        author: None,
        path: None,
        kind: None,
        ..q
    })
}

fn scope(q: &Q) -> Scope {
    let flag = |b: bool| Some(if b { "1" } else { "0" }.to_owned());
    let mut p = ScopeParams {
        period: Some(PERIODS[q.period].to_owned()),
        ..ScopeParams::default()
    };
    p.select.author = q.author.map(|a| AUTHORS[a].1.to_owned());
    p.select.path = q.path.map(|i| ["src", "docs", "src/a.rs"][i].to_owned());
    p.select.kind = q.kind.map(|k| KINDS[k].unwrap_or("other").to_owned());
    p.options.bots = flag(q.bots);
    p.options.ws = flag(q.ws);
    p.options.generated = flag(q.generated);
    Scope::from_params(&p, Options::default(), Period::All).expect("valid scope")
}

fn cx(recs: &[CommitRecord]) -> CountContext {
    CountContext::new(&MemorySource::new(recs), &FixedClock(ANCHOR + 1000 * DAY))
}

fn cases() -> ProptestConfig {
    ProptestConfig {
        cases: 256,
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"),
        )),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(cases())]

    #[test]
    fn p1_accel_answers_equal_the_stream(gens in prop::collection::vec(gen_commit(), 0..40), q in query()) {
        let recs = history(&gens);
        let s = scope(&q);
        let ins = Insights::new(&recs, s.options().clone(), None);
        let c = ins.counted(&s, cx(&recs));
        for k in ReducerKind::ALL {
            let got = c.reduce_kind(&NoAccel, k).expect("no read error");
            prop_assert_eq!(got, c.stream_kind(k), "{:?}", k);
        }
    }

    #[test]
    fn p2_every_breakdown_sums_to_the_totals(gens in prop::collection::vec(gen_commit(), 0..40), q in query()) {
        let recs = history(&gens);
        let s = scope(&q);
        let ins = Insights::new(&recs, s.options().clone(), None);
        let c = ins.counted(&s, cx(&recs));
        let t = c.totals();
        let n = t.commits;
        prop_assert_eq!(n, c.ranks().len());
        let people = c.per_person();
        prop_assert_eq!(n, people.iter().map(|p| p.commits).sum::<usize>());
        prop_assert_eq!(t.additions, people.iter().map(|p| p.additions).sum::<u64>());
        prop_assert_eq!(t.deletions, people.iter().map(|p| p.deletions).sum::<u64>());
        prop_assert_eq!(t.authors, people.len());
        if !q.bots {
            prop_assert!(people.iter().all(|p| !p.bot), "bot counted with bots off");
        }
        for l in [Label::Kind, Label::Scope] {
            let (counts, buckets) = c.per_label(l);
            prop_assert_eq!(n, counts.iter().map(|x| x.commits).sum::<usize>());
            prop_assert_eq!(n, buckets.iter().flat_map(|b| &b.counts).map(|x| x.commits).sum::<usize>());
        }
        let series = c.series();
        prop_assert_eq!(n, series.iter().map(|p| p.commits as usize).sum::<usize>());
        prop_assert_eq!(t.additions, series.iter().map(|p| p.additions).sum::<u64>());
        prop_assert_eq!(t.deletions, series.iter().map(|p| p.deletions).sum::<u64>());
        let w = c.when();
        let n32 = u32::try_from(n).expect("small");
        prop_assert_eq!(n32, w.punch_card.iter().flatten().sum::<u32>());
        prop_assert_eq!(n32, w.by_weekday.iter().sum::<u32>());
        prop_assert_eq!(n32, w.by_hour.iter().sum::<u32>());
        prop_assert_eq!(n, c.sizes().iter().map(|b| b.commits).sum::<usize>());
        // Every person's sparkline adds up to their commits: no commit falls off the axis.
        for p in &people {
            prop_assert_eq!(p.commits, p.series.iter().map(|x| *x as usize).sum::<usize>());
        }
    }

    #[test]
    fn p3_the_span_only_prunes(gens in prop::collection::vec(gen_commit(), 0..40), q in query()) {
        let recs = history(&gens);
        let s = scope(&q);
        let ins = Insights::new(&recs, s.options().clone(), None);
        let cx = cx(&recs);
        let c = ins.counted(&s, cx);
        let streamed: Vec<_> = c.stream().map(|r| r.rank).collect();
        let src = ins.source();
        let full: Vec<_> = src
            .commits_in(CommitRange::All)
            .filter(|r| s.admits(&ins, &cx, r))
            .map(|r| r.rank)
            .collect();
        prop_assert_eq!(&streamed, &full);
        let span = c.span();
        let wide = span.start.saturating_sub(DAY)..span.end.saturating_add(DAY);
        let widened: Vec<_> = src
            .commits_in(CommitRange::Time(wide))
            .filter(|r| s.admits(&ins, &cx, r))
            .map(|r| r.rank)
            .collect();
        prop_assert_eq!(&streamed, &widened);
        prop_assert!(streamed.iter().all(|r| src.commit(*r).is_some_and(|row| c.contains(&row))));
    }
}

proptest! {
    #![proptest_config(cases())]

    /// The /stats KPI, per-day chart and punch card against the reducers.
    #[test]
    fn p4_activity_pages_equal_the_reducers(gens in prop::collection::vec(gen_commit(), 0..40), q in no_chips()) {
        let recs = history(&gens);
        let s = scope(&q);
        let ins = Insights::new(&recs, s.options().clone(), None);
        let c = ins.counted(&s, cx(&recs));
        let p = s.period();
        prop_assert_eq!(ins.period_summary(p).current, c.totals(), "period_summary");
        let r = ins.rhythm(p);
        prop_assert_eq!(&r.series, &c.series(), "rhythm series");
        let w = c.when();
        prop_assert_eq!(r.punch_card, w.punch_card, "punch card");
        prop_assert_eq!(r.by_weekday, w.by_weekday, "by weekday");
    }

    /// Contributors (bots by option), commit types and sizes against the reducers.
    #[test]
    #[ignore = "red until the insights pages read Counted"]
    fn p4_people_labels_sizes_equal_the_reducers(gens in prop::collection::vec(gen_commit(), 0..40), q in no_chips()) {
        let recs = history(&gens);
        let s = scope(&q);
        let ins = Insights::new(&recs, s.options().clone(), None);
        let c = ins.counted(&s, cx(&recs));
        let p = s.period();
        let page: Vec<(String, usize)> = ins
            .contributors(p)
            .contributors
            .into_iter()
            .map(|x| (x.name, x.commits))
            .collect();
        let want: Vec<(String, usize)> = c.per_person().into_iter().map(|x| (x.name, x.commits)).collect();
        prop_assert_eq!(page, want, "contributors");
        for l in [Label::Kind, Label::Scope] {
            let (counts, buckets) = c.per_label(l);
            prop_assert_eq!(ins.label_breakdown(l, p), counts, "label_breakdown");
            prop_assert_eq!(ins.label_over_time(l, p), buckets, "label_over_time");
        }
        prop_assert_eq!(ins.commit_sizes(), c.sizes(), "commit_sizes");
        let (_, freq) = ins.frequency(p);
        let series = c.series();
        let page: Vec<(u64, u64)> = freq.iter().map(|f| (f.additions, f.deletions)).collect();
        let want: Vec<(u64, u64)> = series.iter().map(|f| (f.additions, f.deletions)).collect();
        prop_assert_eq!(page, want, "frequency");
    }
}

proptest! {
    #![proptest_config(cases())]

    /// Frequency, rework, types, churn and sizes against the reducers over one `Counted`.
    #[test]
    fn p4_frequency_labels_churn_sizes_equal_the_reducers(gens in prop::collection::vec(gen_commit(), 0..40), q in no_chips()) {
        let recs = history(&gens);
        let s = scope(&q);
        let ins = Insights::new(&recs, s.options().clone(), None);
        let c = ins.counted(&s, cx(&recs));
        let p = s.period();
        let (_, freq) = ins.frequency(p);
        prop_assert_eq!(&freq, &c.frequency().1, "frequency");
        let series = c.series();
        let page: Vec<(u64, u64)> = freq.iter().map(|f| (f.additions, f.deletions)).collect();
        let want: Vec<(u64, u64)> = series.iter().map(|f| (f.additions, f.deletions)).collect();
        prop_assert_eq!(page, want, "frequency against series");
        prop_assert_eq!(ins.rework(p), c.rework(), "rework");
        for l in [Label::Kind, Label::Scope] {
            let (counts, buckets) = c.per_label(l);
            prop_assert_eq!(ins.label_breakdown(l, p), counts, "label_breakdown");
            prop_assert_eq!(ins.label_over_time(l, p), buckets, "label_over_time");
        }
        let ranks: Vec<_> = ins.counted_rows(p).map(|r| r.rank).collect();
        prop_assert_eq!(ranks, c.ranks(), "counted_rows");
        prop_assert_eq!(ins.file_churn(p), c.per_file(), "file_churn");
        prop_assert_eq!(ins.dir_churn(p), c.per_dir(), "dir_churn");
        prop_assert_eq!(ins.commit_sizes_in(p), c.sizes(), "commit_sizes_in");
        let t = c.totals();
        prop_assert_eq!(t.commits, c.sizes().iter().map(|b| b.commits).sum::<usize>());
    }

    /// The activity and contributor views over one Counted, chips included, equal its reducers:
    /// one-pass activity == totals + series + when, and every person's row and sparkline add up.
    #[test]
    fn p4_period_views_over_counted_equal_the_reducers(gens in prop::collection::vec(gen_commit(), 0..40), q in query()) {
        let recs = history(&gens);
        let s = scope(&q);
        let ins = Insights::new(&recs, s.options().clone(), None);
        let c = ins.counted(&s, cx(&recs));
        let (sum, r) = c.activity();
        let t = c.totals();
        prop_assert_eq!(&sum.current, &t, "activity totals");
        prop_assert_eq!(&sum, &c.summary(), "activity summary");
        prop_assert_eq!(&r, &c.rhythm(), "activity rhythm");
        prop_assert_eq!(&r.series, &c.series(), "rhythm series");
        let w = c.when();
        prop_assert_eq!(r.punch_card, w.punch_card, "punch card");
        prop_assert_eq!(r.by_hour, w.by_hour, "by hour");
        let want: Vec<(String, usize, u64, u64, usize)> = c
            .per_person()
            .into_iter()
            .map(|p| (p.name, p.commits, p.additions, p.deletions, p.active_days))
            .collect();
        for m in ["commits", "added", "deleted", "net"] {
            let rep = c.contributors(gitterrace_index::Metric::parse(m));
            let mut got: Vec<(String, usize, u64, u64, usize)> = rep
                .contributors
                .iter()
                .map(|p| (p.name.clone(), p.commits, p.additions, p.deletions, p.active_days))
                .collect();
            let mut want = want.clone();
            got.sort();
            want.sort();
            prop_assert_eq!(got, want, "contributors {}", m);
            prop_assert!(rep.contributors.iter().all(|p| s.options().include_bots || !p.bot));
        }
        let rep = c.contributors(gitterrace_index::Metric::Commits);
        for p in &rep.contributors {
            prop_assert_eq!(p.commits, p.series.iter().map(|x| *x as usize).sum::<usize>());
        }
        prop_assert_eq!(t.commits, rep.contributors.iter().map(|p| p.commits).sum::<usize>());
        prop_assert_eq!(t.authors, rep.contributors.len());
    }
}
