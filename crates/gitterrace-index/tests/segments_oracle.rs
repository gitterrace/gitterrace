//! The segment set answers like the in-memory oracle over every repository of the golden
//! corpus. The oracle's records come straight from git (`CommitSource::records`, the mailmap of
//! the tip applied), never read back from the cache, so a write or read defect in the set shows
//! here. Compared twice: every source method a page reads, and the insights built over each.
#![expect(clippy::expect_used, reason = "test setup")]

use std::path::Path;
use std::sync::Arc;

use gitterrace_git::Repo;
use gitterrace_index::feed::{
    CommitRange, IdentId, IdentPairs, InsightsSource, MemorySource, PathId, Posting, Rank,
};
use gitterrace_index::{CommitRecord, CommitSource, Index, Insights, Options, Period};
use gitterrace_testkit::matrix::Mailmap;
use gitterrace_testkit::oracle::{FROZEN_NOW, frozen, matrix_repo};
use gitterrace_testkit::{Adversarial, git};

/// The oracle side: records built from git, raw identities plus the tip's mailmap (the one
/// identity rule both sides follow). `pin` pins times as commit rows do at read; the
/// aggregate tables keep raw days (a page clamps where it charts), so they meet an unpinned
/// oracle.
fn from_git(repo: &Repo, order: Option<&[String]>, pin: bool) -> MemorySource<'static> {
    let walked = CommitSource::rev_list(repo, "HEAD").expect("rev list");
    let order = order.unwrap_or(&walked);
    let mut recs: Vec<CommitRecord> = CommitSource::records(repo, order).expect("records");
    let latest = FROZEN_NOW + 86_400;
    let mut pairs: Vec<(String, String)> = Vec::new();
    for r in &mut recs {
        if pin {
            r.time = gitterrace_index::plausible_time(r.time, latest);
        }
        let p = (r.author.clone(), r.email.clone());
        if !pairs.contains(&p) {
            pairs.push(p);
        }
    }
    let mapped = CommitSource::map_identities(repo, "HEAD", &pairs).expect("mailmap");
    let map: IdentPairs = pairs.into_iter().zip(mapped).collect();
    MemorySource::shared_raw(Arc::new(recs), map)
}

/// Every source method, both sides. Ids are the oracle's first-use numbering, which the set
/// keeps (one numbering per cache), so ids are compared directly.
#[expect(clippy::many_single_char_names, reason = "d and m are the two sides throughout")]
fn same_source(
    d: &dyn InsightsSource,
    m: &dyn InsightsSource,
    raw: &dyn InsightsSource,
    case: &str,
) {
    assert_eq!(d.len(), m.len(), "{case}: len");
    let n = u32::try_from(m.len()).expect("len");
    for r in (0..n).map(Rank) {
        assert_eq!(d.commit(r), m.commit(r), "{case}: commit {r:?}");
        assert_eq!(d.commit_id(r), m.commit_id(r), "{case}: id {r:?}");
        assert_eq!(d.parent_ids(r), m.parent_ids(r), "{case}: parents {r:?}");
    }
    let a: Vec<_> = d.commits_in(CommitRange::All).collect();
    let b: Vec<_> = m.commits_in(CommitRange::All).collect();
    assert_eq!(a, b, "{case}: commits");
    let ranks: Vec<Rank> = (0..n).map(Rank).collect();
    let a: Vec<_> = d.rows_for(ranks.clone()).collect();
    let b: Vec<_> = m.rows_for(ranks).collect();
    assert_eq!(a, b, "{case}: rows");
    let a: Vec<_> = d.agg_author_day(i64::MIN..i64::MAX).collect();
    let b: Vec<_> = raw.agg_author_day(i64::MIN..i64::MAX).collect();
    assert_eq!(a, b, "{case}: author days");
    let a: Vec<_> = d.agg_path_author("").collect();
    let b: Vec<_> = raw.agg_path_author("").collect();
    assert_eq!(a, b, "{case}: path authors");
    assert_eq!(d.paths(), m.paths(), "{case}: paths");
    for p in (0..u32::try_from(m.paths()).expect("paths")).map(PathId) {
        assert_eq!(d.path(p), m.path(p), "{case}: path {p:?}");
        let a: Vec<_> = d.postings(Posting::Path(p)).collect();
        let b: Vec<_> = m.postings(Posting::Path(p)).collect();
        assert_eq!(a, b, "{case}: path postings {p:?}");
    }
    assert_eq!(d.idents(), m.idents(), "{case}: idents");
    for i in (0..u32::try_from(m.idents()).expect("idents")).map(IdentId) {
        assert_eq!(d.ident(i), m.ident(i), "{case}: ident {i:?}");
        let a: Vec<_> = d.postings(Posting::Author(i)).collect();
        let b: Vec<_> = m.postings(Posting::Author(i)).collect();
        assert_eq!(a, b, "{case}: author postings {i:?}");
    }
    let a: Vec<_> = d.rename_edges().collect();
    let b: Vec<_> = m.rename_edges().collect();
    assert_eq!(a, b, "{case}: rename edges");
}

/// The insights a page shows, built over each side, through their `Debug` form.
fn same_insights(d: &dyn InsightsSource, m: &dyn InsightsSource, case: &str) {
    for opts in [
        Options::default(),
        Options {
            include_bots: true,
            count_whitespace: true,
            respect_ignore: false,
            ..Options::default()
        },
    ] {
        let (a, b) = (
            Insights::from_source(d, opts.clone(), None),
            Insights::from_source(m, opts.clone(), None),
        );
        let both = |f: &dyn Fn(&Insights<'_>) -> String, what: &str| {
            assert_eq!(f(&a), f(&b), "{case}: {what} under {opts:?}");
        };
        for p in [Period::All, Period::Year, Period::Month, Period::Week] {
            both(&|i| format!("{:?}", i.period_summary(p)), "summary");
            both(&|i| format!("{:?}", i.rhythm(p)), "rhythm");
            both(&|i| format!("{:?}", i.contributors(p)), "contributors");
            both(&|i| format!("{:?}", i.hotspots(&[], p)), "hotspots");
            both(&|i| format!("{:?}", i.ownership(&[], p, 2)), "ownership");
            both(&|i| format!("{:?}", i.commit_sizes_in(p)), "sizes");
        }
        for path in ["a.txt", "src/a.rs", "r.rs"] {
            both(&|i| format!("{:?}", i.file_history(path)), "file history");
        }
    }
}

fn check(repo_dir: &Path, case: &str) {
    let repo = Repo::discover(repo_dir).expect("open");
    let cache = tempfile::tempdir().expect("tmp");
    let index = Index::open_dir(cache.path().join("c")).expect("index");
    index.update(&repo, "HEAD", &|_| {}).expect("update");
    let set = index
        .disk_source(&repo, "HEAD")
        .expect("open set")
        .expect("the set holds HEAD")
        .unchecked();
    let oracle = from_git(&repo, None, true);
    same_source(&*set, &oracle, &from_git(&repo, None, false), case);
    same_insights(&*set, &oracle, case);
    same_lives(&cache.path().join("c"), &repo, None, case);
    assert_eq!(set.failure(), None, "{case}");
}

/// Lives (section 7), life x month (9), life x author (10) and merges per author day (6) of
/// the set equal a replay of the records from git in the set's order.
fn same_lives(cache: &Path, repo: &Repo, order: Option<&[String]>, case: &str) {
    let walked = CommitSource::rev_list(repo, "HEAD").expect("rev list");
    let order = order.unwrap_or(&walked);
    let recs: Vec<CommitRecord> = CommitSource::records(repo, order).expect("records");
    let store = gitterrace_index::seg::read::Store::open(cache).expect("store").expect("a set");
    let (canon, _) = gitterrace_index::seg::canon::dump(&store).expect("canon");
    let (want, _) = gitterrace_index::seg::canon::canon_of(&recs);
    assert_eq!(canon, want, "{case}: lives");
    let got = gitterrace_index::seg::canon::life_tables(&store).expect("life tables");
    let want = gitterrace_index::seg::canon::life_tables_of(&recs);
    assert_eq!(got.months, want.months, "{case}: life months");
    assert_eq!(got.authors, want.authors, "{case}: life authors");
    assert_eq!(got.merges, want.merges, "{case}: merges");
}

/// Every golden corpus repository (the option matrix under each mailmap, the adversarial
/// history and its shallow clone, the rename chain, a merge with its own author) answers like
/// the oracle.
#[test]
fn segments_answer_like_the_oracle_over_the_golden_corpus() {
    if !frozen("segments_answer_like_the_oracle_over_the_golden_corpus") {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    for (k, mailmap) in [Mailmap::AsIs, Mailmap::Edited, Mailmap::CaseFold].into_iter().enumerate()
    {
        let (repo, _) = matrix_repo(&tmp.path().join(format!("m{k}")), mailmap);
        check(&repo, &format!("matrix {k}"));
    }
    let adv = Adversarial::build(&tmp.path().join("adv"));
    check(&adv.repo, "adversarial");
    check(&adv.shallow, "adversarial shallow");
    let chain = gitterrace_testkit::RenameChain::build(&tmp.path().join("ren"));
    check(&chain.repo, "rename chain");
    let merge = gitterrace_testkit::MergeWithAuthor::build(&tmp.path().join("mrg"));
    check(&merge.repo, "merge with author");
}

/// An incremental update (HEAD~2, then HEAD: an unmerged two-segment set) answers like the
/// oracle too.
#[test]
fn an_appended_segment_answers_like_the_oracle() {
    if !frozen("an_appended_segment_answers_like_the_oracle") {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let (path, _) = matrix_repo(&tmp.path().join("m"), Mailmap::Edited);
    let repo = Repo::discover(&path).expect("open");
    let cache = tempfile::tempdir().expect("tmp");
    let index = Index::open_dir(cache.path().join("c")).expect("index");
    let tip = git(&path, &["rev-parse", "HEAD"]);
    git(&path, &["checkout", "-q", "HEAD~2"]);
    index.update(&repo, "HEAD", &|_| {}).expect("first step");
    git(&path, &["checkout", "-q", tip.trim()]);
    index.update(&repo, "HEAD", &|_| {}).expect("second step");
    let set = index.disk_source(&repo, "HEAD").expect("open").expect("set").unchecked();
    // The appended set ranks old order then new commits: the oracle follows the same order.
    let order: Vec<String> = (0..u32::try_from(set.len()).expect("len"))
        .filter_map(|r| set.commit_id(Rank(r)).map(|id| id.to_string()))
        .collect();
    let oracle = from_git(&repo, Some(&order), true);
    let raw = from_git(&repo, Some(&order), false);
    same_source(&*set, &oracle, &raw, "appended");
    same_insights(&*set, &oracle, "appended");
    same_lives(&cache.path().join("c"), &repo, Some(&order), "appended");
}
