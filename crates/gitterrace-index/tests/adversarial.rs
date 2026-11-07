//! Every insight over the adversarial repository: absurd dates, two roots, an octopus merge, a
//! late `.mailmap`, a symlinked `.git/info` and a shallow clone. Each must answer quickly with
//! bounded charts and the same people the log shows.
#![expect(clippy::expect_used, reason = "test setup")]

use std::time::{Duration, Instant};

use gitterrace_git::Repo;
use gitterrace_index::feed::FixedClock;
use gitterrace_index::{
    CountContext, Index, Insights, Label, LineCache, MAX_BUCKETS, Options, Period, Scope,
    ScopeParams, TimeAxis, head_files,
};
use gitterrace_testkit::{Adversarial, git_at};

const PERIODS: [Period; 5] = [
    Period::Week,
    Period::Month,
    Period::Quarter,
    Period::Year,
    Period::All,
];
const FAST: Duration = Duration::from_millis(100);

fn indexed(path: &std::path::Path) -> (Repo, Index) {
    let repo = Repo::discover(path).expect("open");
    let index = Index::open(&repo).expect("index");
    index.update(&repo, "HEAD", &|_| {}).expect("update");
    (repo, index)
}

fn timed<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    assert!(start.elapsed() < FAST, "{what} took {:?}", start.elapsed());
    out
}

/// Runs every insight for every period, each under the time budget with bounded buckets.
fn every_insight(repo: &Repo, ins: &Insights<'_>) {
    let mut lines = LineCache::default();
    let files = head_files(repo, "HEAD", &mut lines).expect("head files");
    for p in PERIODS {
        let r = timed("rhythm", || ins.rhythm(p));
        assert!(r.series.len() <= MAX_BUCKETS, "{p:?}: {}", r.series.len());
        let c = timed("contributors", || ins.contributors(p));
        assert!(c.labels.len() <= MAX_BUCKETS);
        let (_, f) = timed("frequency", || ins.frequency(p));
        assert!(f.len() <= MAX_BUCKETS);
        let l = timed("label over time", || ins.label_over_time(Label::Kind, p));
        assert!(l.len() <= MAX_BUCKETS);
        timed("rework", || ins.rework(p));
        timed("pulse", || ins.period_summary(p));
        timed("churn", || (ins.file_churn(p), ins.dir_churn(p)));
        timed("labels", || ins.label_breakdown(Label::Scope, p));
        timed("hotspots", || ins.hotspots(&files, p));
        timed("ownership", || ins.ownership(&files, p, 2));
    }
    for p in PERIODS {
        timed("sizes", || ins.commit_sizes_in(p));
    }
    timed("age", || ins.code_age(&files));
    timed("history", || ins.file_history("a.txt"));
    timed("cadence", || {
        let s = Scope::from_params(&ScopeParams::default(), Options::default(), Period::All)
            .expect("scope");
        let cx = CountContext::new(ins.source(), &FixedClock(i64::MAX));
        ins.cadence_in(&[], &ins.counted(&s, cx))
    });
}

#[test]
fn every_insight_answers_fast_and_bounded() {
    let tmp = tempfile::tempdir().expect("tmp");
    let fx = Adversarial::build(tmp.path());
    let (repo, index) = indexed(&fx.repo);
    let records = index.records(&repo, "HEAD").expect("records");
    let ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
    for want in [
        &fx.roots[0],
        &fx.roots[1],
        &fx.octopus,
        &fx.far,
        &fx.extreme,
        &fx.tip,
    ] {
        assert!(ids.contains(&want.as_str()), "{want} not indexed");
    }
    assert_eq!(index.pending(&repo, "HEAD").expect("pending"), 0);
    let oct = records
        .iter()
        .find(|r| r.id == fx.octopus)
        .expect("octopus");
    assert!(oct.merge && oct.parents.len() == 3);
    let ins = Insights::new(&records, Options::default(), None);
    every_insight(&repo, &ins);
    // The tip is a real 2024 commit: the future-dated ones must not empty its month.
    assert!(ins.period_summary(Period::Month).current.commits >= 1);
}

/// One commit dated in year 950 million and one at the i64 edges must not size any axis.
#[test]
fn an_axis_never_exceeds_the_bucket_cap() {
    for (first, last) in [
        (0, 3_000_000_000),
        (-3_000_000_000, 0),
        (i64::MIN / 2, i64::MAX / 2),
    ] {
        let start = Instant::now();
        let axis = TimeAxis::for_period(None, first, last);
        assert!(
            axis.len() <= MAX_BUCKETS && !axis.is_empty(),
            "{first}..{last}"
        );
        assert!(start.elapsed() < FAST);
    }
}

/// The `.mailmap` at the tip applies to all history, and an edit reaches cached records.
#[test]
fn the_tip_mailmap_names_every_cached_commit() {
    let tmp = tempfile::tempdir().expect("tmp");
    let fx = Adversarial::build(tmp.path());
    let (repo, index) = indexed(&fx.repo);
    let records = index.records(&repo, "HEAD").expect("records");
    let ins = Insights::new(&records, Options::default(), None);
    let people = &ins.contributors(Period::All).contributors;
    assert_eq!(people.len(), 1, "{people:?}");
    let log = repo.log("HEAD", None, 0, 100).expect("log");
    for r in &records {
        let shown = log.iter().find(|c| c.id == r.id).expect("in log");
        assert_eq!(r.author, shown.author.name, "{}", r.id);
    }
    std::fs::write(
        fx.repo.join(".mailmap"),
        "Jane D. <jane@x.io> <old@x.io>\nJane D. <jane@x.io> <jane@x.io>\n",
    )
    .expect("write mailmap");
    git_at(
        &fx.repo,
        "2024-04-01T12:00:00Z",
        &["commit", "-qam", "docs: rename"],
    );
    index.update(&repo, "HEAD", &|_| {}).expect("update");
    let records = index.records(&repo, "HEAD").expect("records");
    assert!(
        records.iter().all(|r| r.author == "Jane D."),
        "{:?}",
        records.iter().map(|r| &r.author).collect::<Vec<_>>()
    );
}

/// A shallow clone indexes completely, its boundary diffed as a root, and every insight holds.
#[test]
fn a_shallow_clone_indexes_its_boundary_as_a_root() {
    let tmp = tempfile::tempdir().expect("tmp");
    let fx = Adversarial::build(tmp.path());
    let (repo, index) = indexed(&fx.shallow);
    assert_eq!(index.pending(&repo, "HEAD").expect("pending"), 0);
    let records = index.records(&repo, "HEAD").expect("records");
    assert_eq!(records.len(), 3);
    let edge = records.iter().find(|r| r.id == fx.far).expect("boundary");
    assert!(!edge.files.is_empty(), "the boundary adds its whole tree");
    assert!(
        repo.commit(&fx.far).is_ok(),
        "the boundary's commit page opens"
    );
    let ins = Insights::new(&records, Options::default(), None);
    every_insight(&repo, &ins);
}
