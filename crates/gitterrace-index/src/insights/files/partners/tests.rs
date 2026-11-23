//! The coupling report against a brute-force recount of the commits.

use std::collections::BTreeMap;

use super::{COUPLING_MAX_FILES, CouplingQuery, CouplingReport};
use crate::insights::HeadFile;
use crate::insights::fixtures::{file, rec};
use crate::{CommitRecord, Insights, Options, Period};

/// A history with sibling pairs, cross-directory pairs, one sweep and a lone file.
fn history() -> (Vec<CommitRecord>, Vec<HeadFile>) {
    let mut recs = Vec::new();
    let mut at = 0;
    let mut add = |recs: &mut Vec<CommitRecord>, who: &str, names: &[&str]| {
        at += 1;
        let files = names.iter().map(|n| file(n, 1, 0, 5)).collect();
        recs.push(rec(&format!("c{at}"), who, at, files));
    };
    for n in 0..6 {
        add(&mut recs, "Ann", &["src/a.rs", "src/b.rs"]);
        if n % 2 == 0 {
            add(&mut recs, "Bob", &["src/a.rs", "tests/a_test.rs"]);
        }
        if n % 3 == 0 {
            add(&mut recs, "Bob", &["src/b.rs", "src/c.rs", "docs/x.md"]);
        }
    }
    add(&mut recs, "Ann", &["lone.rs"]);
    let sweep: Vec<String> = (0..=COUPLING_MAX_FILES).map(|i| format!("gen/f{i}.rs")).collect();
    let names: Vec<&str> = sweep.iter().map(String::as_str).collect();
    add(&mut recs, "Ann", &names);
    let mut head: Vec<HeadFile> = ["src/a.rs", "src/b.rs", "src/c.rs", "tests/a_test.rs"]
        .iter()
        .chain(&["docs/x.md", "lone.rs"])
        .map(|p| HeadFile { path: (*p).into(), lines: 5 })
        .collect();
    head.extend(sweep.iter().map(|p| HeadFile { path: p.clone(), lines: 5 }));
    (recs, head)
}

/// Pairs recounted from the records alone: no tip index, no walk, no budget.
fn brute(recs: &[CommitRecord], head: &[HeadFile]) -> (BTreeMap<(String, String), usize>, usize) {
    let known = |p: &str| head.iter().any(|h| h.path == p);
    let mut pairs = BTreeMap::new();
    let mut skipped = 0;
    for r in recs {
        let mut names: Vec<&str> = r.files.iter().map(|f| &*f.path).filter(|p| known(p)).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() > COUPLING_MAX_FILES {
            skipped += 1;
        }
        if !(2..=COUPLING_MAX_FILES).contains(&names.len()) {
            continue;
        }
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                *pairs.entry(((*a).to_owned(), (*b).to_owned())).or_default() += 1;
            }
        }
    }
    (pairs, skipped)
}

fn report(ins: &Insights<'_>, head: &[HeadFile], q: CouplingQuery<'_>) -> CouplingReport {
    ins.in_period(Period::All, |c| ins.coupling_report_in(head, c, &q))
}

fn query(path: Option<&str>, min: usize, cross: bool) -> CouplingQuery<'_> {
    CouplingQuery { path, min_support: min, cross_dir: cross, limit: usize::MAX }
}

#[test]
fn the_global_list_equals_a_brute_force_recount() {
    let (recs, head) = history();
    let ins = Insights::new(&recs, Options::default(), None);
    let (want, skipped) = brute(&recs, &head);
    for min in [1, 2, 3] {
        let got = report(&ins, &head, query(None, min, true));
        let counted: BTreeMap<(String, String), usize> =
            got.pairs.iter().map(|p| ((p.a.clone(), p.b.clone()), p.together)).collect();
        let kept: BTreeMap<(String, String), usize> =
            want.iter().filter(|(_, n)| **n >= min).map(|(k, n)| (k.clone(), *n)).collect();
        assert_eq!(counted, kept, "min_support {min}");
        assert_eq!(got.skipped_commits, skipped);
        assert_eq!(got.max_commit_files, COUPLING_MAX_FILES);
    }
}

#[test]
fn a_file_view_equals_the_global_list_filtered_to_that_file() {
    let (recs, head) = history();
    let ins = Insights::new(&recs, Options::default(), None);
    for cross in [false, true] {
        let all = report(&ins, &head, query(None, 1, cross));
        for h in &head {
            let one = report(&ins, &head, query(Some(&h.path), 1, cross));
            let want: Vec<_> =
                all.pairs.iter().filter(|p| p.a == h.path || p.b == h.path).cloned().collect();
            assert_eq!(one.pairs, want, "{} cross {cross}", h.path);
            assert!(one.matrix.is_empty(), "a focus file has no matrix");
        }
    }
}

#[test]
fn a_small_limit_keeps_the_strongest_pairs_of_the_full_list() {
    let (recs, head) = history();
    let ins = Insights::new(&recs, Options::default(), None);
    let all = report(&ins, &head, query(None, 1, true));
    for limit in [0, 1, 2, 3] {
        let q = CouplingQuery { limit, ..query(None, 1, true) };
        let got = report(&ins, &head, q);
        assert_eq!(got.pairs, all.pairs.iter().take(limit).cloned().collect::<Vec<_>>());
    }
}

#[test]
fn sibling_only_leaves_out_cross_directory_pairs() {
    let (recs, head) = history();
    let ins = Insights::new(&recs, Options::default(), None);
    let sib = report(&ins, &head, query(None, 1, false));
    let all = report(&ins, &head, query(None, 1, true));
    assert!(sib.pairs.iter().all(|p| !p.cross_dir));
    assert!(all.pairs.iter().any(|p| p.cross_dir));
    let same: Vec<_> = all.pairs.iter().filter(|p| !p.cross_dir).cloned().collect();
    assert_eq!(sib.pairs, same);
}

#[test]
fn confidence_degree_and_last_together_follow_the_counts() {
    let (recs, head) = history();
    let ins = Insights::new(&recs, Options::default(), None);
    let got = report(&ins, &head, query(Some("src/a.rs"), 1, true));
    let ab = got.pairs.iter().find(|p| p.b == "src/b.rs").expect("a pair with b");
    assert_eq!((ab.together, ab.a_changes, ab.b_changes), (6, 9, 8));
    assert!((ab.confidence_ab - 6.0 / 9.0).abs() < 1e-9);
    assert!((ab.confidence_ba - 6.0 / 8.0).abs() < 1e-9);
    assert!((ab.degree - 6.0 / 8.0).abs() < 1e-9);
    let newest = recs
        .iter()
        .filter(|r| {
            let has = |n: &str| r.files.iter().any(|f| &*f.path == n);
            has("src/a.rs") && has("src/b.rs")
        })
        .map(|r| r.time)
        .max();
    assert_eq!(Some(ab.last_together), newest);
}

#[test]
fn the_matrix_counts_commits_per_top_level_directory_pair() {
    let (recs, head) = history();
    let ins = Insights::new(&recs, Options::default(), None);
    let got = report(&ins, &head, query(None, 1, true));
    let find = |a: &str, b: &str| got.matrix.iter().find(|m| m.a == a && m.b == b);
    assert_eq!(find("src", "tests").map(|m| m.together), Some(3));
    assert_eq!(find("docs", "src").map(|m| m.together), Some(2));
    assert!(got.matrix.iter().all(|m| m.a < m.b));
}
