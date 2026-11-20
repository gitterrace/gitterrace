use std::sync::Arc;

use axum::extract::{Query, State};

use crate::search::commits::search_commits_counted;
use crate::search::{
    SearchQuery, build_regex, file_hits, glob_regex, highlight, memo_key, search, search_code,
};

/// Guard: a repeat search at the same tip is served from the memo, not rescanned. The
/// memo entry is swapped for a marker; a handler that scans again would not return it.
#[tokio::test]
async fn repeat_search_at_one_tip_is_memoized() {
    let dir = tempfile::tempdir().expect("tmp");
    let f =
        gitterrace_testkit::Fixture::init(dir.path()).write("a.txt", "needle\n").commit("feat: a");
    let app = crate::tests::new_app(f.path());
    let q = || SearchQuery { q: Some("needle".into()), ..SearchQuery::default() };
    let first = search(State(Arc::clone(&app)), Query(q())).await.expect("page").0;
    assert!(first.contains("1 match"), "{first}");
    app.memo_put(memo_key(&app, &q()), "memo");
    let again = search(State(Arc::clone(&app)), Query(q())).await.expect("page").0;
    assert_eq!(again, "memo", "the repeat must not scan again");
    let other = SearchQuery { case: Some("on".into()), ..q() };
    let other = search(State(app), Query(other)).await.expect("page").0;
    assert_ne!(other, "memo", "options are part of the key");
}

/// Guard: a capped code search keeps the first matching files in path order, whatever
/// the thread timing, so the memoized page is the same page a rescan would give.
#[test]
fn capped_search_keeps_the_first_files_in_path_order() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut f = gitterrace_testkit::Fixture::init(dir.path());
    for i in 0..600 {
        f = f.write(&format!("f{i:04}.txt"), "needle\nneedle\n");
    }
    let f = f.commit("feat: many");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let mut cfg = gitterrace_config::Config::default();
    cfg.limits.search_matches = 501;
    let app = crate::app::App::configured(repo, index, &crate::app::Env::new(cfg));
    let re = build_regex("needle", false, false).expect("re");
    for _ in 0..5 {
        let out = search_code(&app, "HEAD", &re, None).expect("scan");
        let paths: Vec<&str> = out.files.iter().map(|h| h.path.as_str()).collect();
        let want: Vec<String> = (0..251).map(|i| format!("f{i:04}.txt")).collect();
        assert_eq!(paths, want, "the first files, in order");
        assert_eq!((out.matches, out.capped), (501, true));
        assert_eq!(out.files.last().map(|h| h.count), Some(1), "the last file is cut");
        assert_eq!(out.scanned, 251, "files up to the one that hits the cap count as scanned");
    }
}

#[test]
fn literal_is_escaped_and_case_folds() {
    let re = build_regex("a.b", false, false).expect("re");
    assert!(re.is_match("xA.Bx"));
    assert!(!re.is_match("axb"));
    let re = build_regex("a.b", false, true).expect("re");
    assert!(!re.is_match("A.B"));
    assert!(build_regex("(", true, false).is_err());
}

#[test]
fn globs_match_names_and_paths() {
    let g = glob_regex("*.rs, docs/").expect("g").expect("some");
    assert!(g.is_match("crates/a/src/x.rs"));
    assert!(g.is_match("docs/a/b.md"));
    assert!(!g.is_match("x.md"));
    let g = glob_regex("src/**/*.rs").expect("g").expect("some");
    assert!(g.is_match("src/a/b.rs") && g.is_match("src/b.rs"));
    assert!(glob_regex("").expect("g").is_none());
}

#[test]
fn highlights_escape_and_mark() {
    let re = build_regex("b", false, false).expect("re");
    assert_eq!(highlight("a<b>b", &re), "a&lt;<mark>b</mark>&gt;<mark>b</mark>");
}

#[test]
fn hits_carry_context_and_gaps() {
    let re = build_regex("x", false, true).expect("re");
    let text = "x\n.\n.\n.\n.\nx\n.\n";
    let h = file_hits("a", "HEAD", text, &re, 100);
    assert_eq!(h.count, 2);
    let ns: Vec<usize> = h.lines.iter().map(|l| l.n).collect();
    assert_eq!(ns, vec![1, 2, 0, 5, 6, 7]);
}

#[test]
fn a_file_header_counts_its_matching_lines_with_the_noun() {
    let re = build_regex("x", false, true).expect("re");
    assert_eq!(file_hits("a", "HEAD", "x\n", &re, 100).matches(), "1 match");
    assert_eq!(file_hits("a", "HEAD", "x\nx x\n.\n", &re, 100).matches(), "2 matches");
}

/// Guard: a commit search that matches nothing builds no summary, so a miss costs the walk
/// alone and never more than a hit over the same history. A search that builds the summary
/// of every commit before testing it fails here; so does one that rereads a commit per match.
#[test]
fn a_commit_miss_builds_no_more_than_a_hit() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut f = gitterrace_testkit::Fixture::init(dir.path());
    for i in 0..30 {
        f = f.write("a.txt", &format!("{i}\n")).commit(&format!("chore: step {i}"));
    }
    let f = f.write("a.txt", "end\n").commit("feat: needle");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = crate::app::App::configured(repo, index, &env);
    let run = |q: &str| {
        let re = build_regex(q, false, false).expect("re");
        search_commits_counted(&app, "HEAD", &re).expect("scan")
    };
    let (hit_rows, _, hit) = run("needle");
    let (miss_rows, capped, miss) = run("zzqq-nothing");
    assert_eq!(hit_rows.len(), 1);
    assert_eq!(hit_rows.first().map(|c| c.subject.as_str()), Some("feat: needle"));
    assert!(miss_rows.is_empty() && !capped);
    assert_eq!((hit.walked, miss.walked), (31, 31), "both walk the whole history");
    assert_eq!(miss.built, 0, "a miss builds no summary");
    assert!(miss.built <= hit.built, "{miss:?} vs {hit:?}");
    let (rows, _, all) = run("step");
    assert_eq!((rows.len(), all.built), (30, 30), "one summary per match");
}

/// Guard class: a request-sized search input is refused with a message before it is scanned,
/// memoized or echoed in full. Red when an over-cap query or glob runs, or the page repeats it.
#[tokio::test]
async fn an_over_long_query_or_filter_is_refused_with_a_message() {
    let dir = tempfile::tempdir().expect("tmp");
    let f =
        gitterrace_testkit::Fixture::init(dir.path()).write("a.txt", "needle\n").commit("feat: a");
    let app = crate::tests::new_app(f.path());
    let long = "needle".repeat(crate::search::MAX_QUERY_BYTES);
    let page = search(
        State(Arc::clone(&app)),
        Query(SearchQuery { q: Some(long.clone()), ..SearchQuery::default() }),
    )
    .await
    .expect("page")
    .0;
    assert!(page.contains("query is longer than 1024 bytes"), "{page}");
    assert!(!page.contains("1 match"), "an over-cap query does not run");
    // A page holds no run of the query longer than the cap: its size grows with the chrome.
    assert!(
        !page.contains(&long[..=crate::search::MAX_QUERY_BYTES]),
        "the query is not echoed in full"
    );
    let page = search(
        State(Arc::clone(&app)),
        Query(SearchQuery {
            q: Some("needle".into()),
            glob: Some("*.txt,".repeat(400)),
            ..SearchQuery::default()
        }),
    )
    .await
    .expect("page")
    .0;
    assert!(page.contains("file filter is longer than 1024 bytes"), "{page}");
    let at_cap = "n".repeat(crate::search::MAX_QUERY_BYTES);
    let page = search(State(app), Query(SearchQuery { q: Some(at_cap), ..SearchQuery::default() }))
        .await
        .expect("page")
        .0;
    assert!(!page.contains("longer than"), "a query at the cap runs");
}

#[test]
fn an_oversized_glob_is_refused_like_an_oversized_pattern() {
    // Past the 8 MiB the query regex allows, within the regex crate's own default.
    let glob = "?".repeat(10_000);
    assert!(glob_regex(&glob).is_err(), "the glob regex has the query's size limit");
    assert!(glob_regex("src/**/*.rs, *.md").expect("small").is_some());
}

#[test]
fn scan_threads_stay_within_blocking_jobs() {
    let cfg = |jobs| {
        let mut c = gitterrace_config::Config::default();
        c.resources.blocking_jobs = jobs;
        c
    };
    assert_eq!(crate::search::scan_workers(&cfg(1)), 1);
    assert!(crate::search::scan_workers(&cfg(2)) <= 2);
    assert!(crate::search::scan_workers(&cfg(64)) <= 8, "never more than eight");
}

/// N identical cold searches at once scan the tree once: the rest wait and read the memo.
#[tokio::test]
async fn concurrent_cold_identical_searches_scan_once() {
    let dir = tempfile::tempdir().expect("tmp");
    let f =
        gitterrace_testkit::Fixture::init(dir.path()).write("a.txt", "needle\n").commit("feat: a");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let git_dir = repo.git_dir().to_owned();
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = Arc::new(crate::app::App::configured(repo, index, &env));
    let tasks: Vec<_> = (0..6)
        .map(|_| {
            let q = SearchQuery { q: Some("needle".into()), ..SearchQuery::default() };
            tokio::spawn(search(State(Arc::clone(&app)), Query(q)))
        })
        .collect();
    let mut pages = Vec::new();
    for t in tasks {
        pages.push(t.await.expect("task").expect("page").0);
    }
    assert!(pages.iter().all(|p| *p == pages[0]), "one page for all");
    assert_eq!(crate::search::scans_of(&git_dir), 1, "one scan for six requests");
}
