use std::sync::Arc;

use axum::extract::{Query, State};

use crate::search::{
    SearchQuery, build_regex, file_hits, glob_regex, highlight, memo_key, search, search_code,
};

/// Guard: a repeat search at the same tip is served from the memo, not rescanned. The
/// memo entry is swapped for a marker; a handler that scans again would not return it.
#[tokio::test]
async fn repeat_search_at_one_tip_is_memoized() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = gitterrace_testkit::Fixture::init(dir.path())
        .write("a.txt", "needle\n")
        .commit("feat: a");
    let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = Arc::new(crate::app::App::configured(repo, index, &env));
    let q = || SearchQuery {
        q: Some("needle".into()),
        ..SearchQuery::default()
    };
    let first = search(State(Arc::clone(&app)), Query(q()))
        .await
        .expect("page")
        .0;
    assert!(first.contains("1 match"), "{first}");
    app.memo_put(memo_key(&app, &q()), Arc::new("memo".into()));
    let again = search(State(Arc::clone(&app)), Query(q()))
        .await
        .expect("page")
        .0;
    assert_eq!(again, "memo", "the repeat must not scan again");
    let other = SearchQuery {
        case: Some("on".into()),
        ..q()
    };
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
        assert_eq!(
            out.files.last().map(|h| h.count),
            Some(1),
            "the last file is cut"
        );
        assert_eq!(
            out.scanned, 251,
            "files up to the one that hits the cap count as scanned"
        );
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
    assert_eq!(
        highlight("a<b>b", &re),
        "a&lt;<mark>b</mark>&gt;<mark>b</mark>"
    );
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
    assert_eq!(
        file_hits("a", "HEAD", "x\nx x\n.\n", &re, 100).matches(),
        "2 matches"
    );
}
