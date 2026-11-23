//! Latency ceilings of the file pages over the segment set, on a 100k-commit synthetic history.
//!
//! Hotspots and ownership read every touch of every head file. On a large history the cost is
//! the read path (commit columns, one rows pass, rename follow, path filter), not the page.
//! This measures them as the first visitor after an index build pays for them: a fresh
//! source, tables built for the request, period all and one year. The ceilings are the
//! measured times with headroom, so a regression of the read path fails here before a bench.
//!
//! Release builds only (debug is ten times slower and says nothing). Runs in `make check`
//! through `cargo test --release`.
#![cfg(not(debug_assertions))]

use std::io::Write as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use gitterrace_git::Repo;
use gitterrace_index::{
    CouplingQuery, Index, Insights, LineCache, Options, Period, RiskQuery, Scope, ScopeParams,
    head_files,
};
use gitterrace_testkit::synthetic::{self, Shape};

/// One timed test at a time: two 100k builds side by side slow each other past a ceiling.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Ceilings, in ms, for (hotspots, ownership): measured on an M1 Max (all: 224 and 156, one
/// year: 121 and 118, load 55 to 85), times 1.5.
const CEILING_ALL: (u64, u64) = (CEIL_HOT_ALL, CEIL_OWN_ALL);
const CEILING_YEAR: (u64, u64) = (CEIL_HOT_YEAR, CEIL_OWN_YEAR);
const CEIL_HOT_ALL: u64 = 340;
const CEIL_OWN_ALL: u64 = 240;
const CEIL_HOT_YEAR: u64 = 190;
const CEIL_OWN_YEAR: u64 = 180;

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[test]
fn file_pages_stay_under_their_ceilings_at_100k_commits() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("r");
    synthetic::build(&path, Shape::sized(100_000));
    let repo = Repo::discover(&path).expect("repo");
    let index = Index::open_dir(dir.path().join("cache")).expect("index");
    index.update(&repo, "HEAD", &|_| {}).expect("update");
    let tip = repo.resolve("HEAD").expect("tip");
    let mut lines = LineCache::load(index.dir());
    let files = head_files(&repo, &tip, &mut lines).expect("head files");
    let mut report = Vec::new();
    for (period, (hot_max, own_max)) in [(Period::All, CEILING_ALL), (Period::Year, CEILING_YEAR)] {
        // A fresh source and fresh tables per page: what the first request after a build pays.
        // The best of three: a loaded machine slows a run, it never speeds one up.
        let (mut hot_ms, mut own_ms) = (u64::MAX, u64::MAX);
        for _ in 0..3 {
            let src = index
                .disk_source(&repo, &tip)
                .expect("open")
                .expect("the built tip reads the segment set")
                .unchecked();
            let t = Instant::now();
            let ins = Insights::from_source(&*src, Options::default(), None);
            let hot = ins.hotspots(&files, period);
            hot_ms = hot_ms.min(ms(t.elapsed()));
            let src = index
                .disk_source(&repo, &tip)
                .expect("open")
                .expect("the built tip reads the segment set")
                .unchecked();
            let t = Instant::now();
            let ins = Insights::from_source(&*src, Options::default(), None);
            let own = ins.ownership(&files, period, 1);
            own_ms = own_ms.min(ms(t.elapsed()));
            assert!(!hot.files.is_empty() && own.overall.lines > 0, "pages are not empty");
        }
        report.push(format!("{period:?}: hotspots {hot_ms} ms, ownership {own_ms} ms"));
        assert!(
            hot_ms <= hot_max && own_ms <= own_max,
            "{period:?}: hotspots {hot_ms} ms (ceiling {hot_max}), ownership {own_ms} ms \
             (ceiling {own_max}) over {} head files",
            files.len()
        );
    }
    // The report is the point of a passing run: the times against their ceilings.
    writeln!(std::io::stderr(), "{}", report.join("\n")).expect("stderr");
}

/// Ceilings, in ms, for one file card and one directory card: a card reads the whole hotspots,
/// ownership and coupling reports to rank its path, plus the touches of its own files.
/// Measured on an M1 Max at load 9 to 12, times 1.5: file 220, directory of 1.1k files at the
/// card's chunk of 256 paths 262 (chunk 64: 2543, one visit: 129). A file card reads the whole
/// hotspots report, so its ceiling matches `CEIL_HOT_ALL`.
const CEIL_FILE_CARD: u64 = 340;
const CEIL_DIR_CARD: u64 = 400;
/// Chunk sizes the report compares: 64 (the postings bound), 256 (the card's) and one visit.
const CHUNKS: [usize; 3] = [64, 256, 0];

#[test]
fn path_cards_stay_under_their_ceilings_at_100k_commits() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("r");
    synthetic::build(&path, Shape::sized(100_000));
    let repo = Repo::discover(&path).expect("repo");
    let index = Index::open_dir(dir.path().join("cache")).expect("index");
    index.update(&repo, "HEAD", &|_| {}).expect("update");
    let tip = repo.resolve("HEAD").expect("tip");
    let mut lines = LineCache::load(index.dir());
    let files = head_files(&repo, &tip, &mut lines).expect("head files");
    let scope = Scope::from_params(&ScopeParams::default(), Options::default(), Period::All)
        .expect("scope");
    // One file, and the widest top-level directory of the synthetic tree (about a 37th of it).
    let cases: [(&str, &str, usize); 3] =
        [("file", "d0/d0/f0.rs", 64), ("dir", "d0", 256), ("root", "", 64)];
    let mut report = Vec::new();
    let mut ceilings = (0, 0);
    for (what, at, chunk) in cases.into_iter().take(2) {
        let best = best_of_three(&index, &repo, &tip, |ins| {
            let c = ins.counted(&scope, ins.context());
            let card = ins.path_card_chunked(&files, &c, at, chunk);
            assert!(card.changes > 0, "{at} has changes");
        })
        .expect("the built tip opens");
        match what {
            "file" => ceilings.0 = best,
            _ => ceilings.1 = best,
        }
        report.push(format!("{what} card {at}: {best} ms (chunk {chunk})"));
    }
    for chunk in CHUNKS {
        let best = best_of_three(&index, &repo, &tip, |ins| {
            let c = ins.counted(&scope, ins.context());
            ins.path_card_chunked(&files, &c, "d0", chunk);
        })
        .expect("the built tip opens");
        report.push(format!("dir card d0 at chunk {chunk}: {best} ms"));
    }
    writeln!(std::io::stderr(), "{}", report.join("\n")).expect("stderr");
    assert!(
        ceilings.0 <= CEIL_FILE_CARD && ceilings.1 <= CEIL_DIR_CARD,
        "file card {} ms (ceiling {CEIL_FILE_CARD}), directory card {} ms (ceiling \
         {CEIL_DIR_CARD}) over {} head files",
        ceilings.0,
        ceilings.1,
        files.len()
    );
}

/// Ceilings, in ms, for the global coupling list and one file's partners over the whole
/// history: measured on an M1 Max at load 9 to 17 (124 and 82), with room for load.
/// Quadratic pair growth (every pair of every commit, or a row per file per commit) would put
/// either far above them.
const CEIL_COUPLING_ALL: u64 = 300;
const CEIL_COUPLING_FILE: u64 = 200;

#[test]
fn coupling_stays_under_its_ceilings_at_100k_commits() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("r");
    synthetic::build(&path, Shape::sized(100_000));
    let repo = Repo::discover(&path).expect("repo");
    let index = Index::open_dir(dir.path().join("cache")).expect("index");
    index.update(&repo, "HEAD", &|_| {}).expect("update");
    let tip = repo.resolve("HEAD").expect("tip");
    let mut lines = LineCache::load(index.dir());
    let files = head_files(&repo, &tip, &mut lines).expect("head files");
    let scope = Scope::from_params(&ScopeParams::default(), Options::default(), Period::All)
        .expect("scope");
    let run = |path: Option<&str>| {
        best_of_three(&index, &repo, &tip, |ins| {
            let c = ins.counted(&scope, ins.context());
            let q = CouplingQuery { path, min_support: 3, cross_dir: true, limit: 50 };
            let r = ins.coupling_report_in(&files, &c, &q);
            assert!(r.pairs.len() <= 50);
        })
        .expect("the built tip opens")
    };
    let (all, one) = (run(None), run(Some("d0/d0/f0.rs")));
    writeln!(std::io::stderr(), "coupling all {all} ms, file {one} ms").expect("stderr");
    assert!(
        all <= CEIL_COUPLING_ALL && one <= CEIL_COUPLING_FILE,
        "coupling {all} ms (ceiling {CEIL_COUPLING_ALL}), one file {one} ms (ceiling \
         {CEIL_COUPLING_FILE}) over {} head files",
        files.len()
    );
}

/// Ceilings, in ms, for the knowledge risk of the whole tree and of one person over the whole
/// history: measured on an M1 Max at load 6 (124 and 125), with room for load. Work that grows
/// with the files held per request, or a rescan per person, would put either far above them.
const CEIL_RISK_ALL: u64 = 250;
const CEIL_RISK_PERSON: u64 = 250;

#[test]
fn knowledge_risk_stays_under_its_ceilings_at_100k_commits() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("r");
    synthetic::build(&path, Shape::sized(100_000));
    let repo = Repo::discover(&path).expect("repo");
    let index = Index::open_dir(dir.path().join("cache")).expect("index");
    index.update(&repo, "HEAD", &|_| {}).expect("update");
    let tip = repo.resolve("HEAD").expect("tip");
    let mut lines = LineCache::load(index.dir());
    let files = head_files(&repo, &tip, &mut lines).expect("head files");
    let scope = Scope::from_params(&ScopeParams::default(), Options::default(), Period::All)
        .expect("scope");
    let run = |author: Option<&str>| {
        best_of_three(&index, &repo, &tip, |ins| {
            let c = ins.counted(&scope, ins.context());
            let q = RiskQuery { path: None, author, limit: 50 };
            let r = ins.knowledge_risk_in(&files, &c, &q).expect("known author");
            assert!(r.orphans.len() <= 50);
        })
        .expect("the built tip opens")
    };
    let who = {
        let set = index.disk_source(&repo, &tip).expect("open").expect("set");
        let src = set.unchecked();
        let ins = Insights::from_source(&*src, Options::default(), None);
        let c = ins.counted(&scope, ins.context());
        let top = c.contributors(gitterrace_index::Metric::Commits).contributors;
        top.first().map(|p| p.name.clone()).expect("a contributor")
    };
    let (all, one) = (run(None), run(Some(&who)));
    writeln!(std::io::stderr(), "knowledge risk all {all} ms, person {one} ms").expect("stderr");
    assert!(
        all <= CEIL_RISK_ALL && one <= CEIL_RISK_PERSON,
        "knowledge risk {all} ms (ceiling {CEIL_RISK_ALL}), one person {one} ms (ceiling \
         {CEIL_RISK_PERSON}) over {} head files",
        files.len()
    );
}

/// The best of three runs of `f` over a fresh source and fresh tables each: what the first
/// request after a build pays. A loaded machine slows a run, it never speeds one up.
fn best_of_three(
    index: &Index,
    repo: &Repo,
    tip: &str,
    f: impl Fn(&Insights<'_>),
) -> std::io::Result<u64> {
    let mut best = u64::MAX;
    for _ in 0..3 {
        let Some(set) = index.disk_source(repo, tip)? else {
            return Err(std::io::Error::other("the built tip reads no segment set"));
        };
        let src = set.unchecked();
        let t = Instant::now();
        let ins = Insights::from_source(&*src, Options::default(), None);
        f(&ins);
        best = best.min(ms(t.elapsed()));
    }
    Ok(best)
}
