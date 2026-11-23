use std::collections::BTreeSet;

use gitterrace_testkit::{Fixture, git};
use gitterrace_types::api::PlanTerms;

use super::{Inputs, PlanRequest, glob, has_sibling, is_test, order, terms};
use crate::ApiState;
use crate::insights::Params;

fn inputs() -> Inputs {
    Inputs {
        changed_lines: 0,
        rank: None,
        sole_owner: false,
        bus_factor_one: false,
        untested: false,
        fold_lines: 400,
    }
}

/// The formula of the contract, one row per rule: the points each term gives.
#[test]
fn terms_follow_the_formula_for_each_rule() {
    let table: [(&str, Inputs, [u32; 4]); 8] = [
        ("nothing", inputs(), [0, 0, 0, 0]),
        ("hottest of ten", Inputs { rank: Some((1, 10)), ..inputs() }, [30, 0, 0, 0]),
        ("last of ten", Inputs { rank: Some((10, 10)), ..inputs() }, [3, 0, 0, 0]),
        ("half the fold", Inputs { changed_lines: 200, ..inputs() }, [0, 13, 0, 0]),
        ("past the fold", Inputs { changed_lines: 4000, ..inputs() }, [0, 25, 0, 0]),
        (
            "sole owner",
            Inputs { sole_owner: true, bus_factor_one: true, ..inputs() },
            [0, 0, 25, 0],
        ),
        ("bus factor one", Inputs { bus_factor_one: true, ..inputs() }, [0, 0, 12, 0]),
        ("untested", Inputs { untested: true, ..inputs() }, [0, 0, 0, 20]),
    ];
    for (name, i, [churn, size, ownership, untested]) in table {
        let t = terms(&i);
        assert_eq!(t, PlanTerms { churn, size, ownership, untested }, "{name}");
        assert_eq!(t.sum(), churn + size + ownership + untested, "{name}");
    }
    let most = terms(&Inputs {
        changed_lines: 9999,
        rank: Some((1, 1)),
        sole_owner: true,
        bus_factor_one: true,
        untested: true,
        fold_lines: 400,
    });
    assert_eq!(most.sum(), 100);
}

fn row(path: &str, risk: u32, lines: u32) -> gitterrace_types::api::PlanFile {
    let t = PlanTerms { churn: risk, ..PlanTerms::default() };
    gitterrace_types::api::PlanFile {
        additions: lines,
        binary: false,
        bus_factor_one: false,
        deletions: 0,
        generated: false,
        has_test_sibling: false,
        hotspot: None,
        is_test: false,
        kind: gitterrace_types::ChangeKind::Modified,
        link: None,
        lockfile: false,
        old_path: None,
        owners: Vec::new(),
        path: path.to_owned(),
        permalink: None,
        risk: t.sum(),
        size_lines: lines,
        sole_owner: false,
        terms: t,
    }
}

/// Risk, then the bigger change, then the path: whatever the input order, the same output.
#[test]
fn order_is_risk_then_size_then_path_whatever_the_input_order() {
    let rows =
        || vec![row("c", 10, 1), row("b", 10, 5), row("a", 10, 5), row("z", 20, 1), row("y", 0, 9)];
    let mut forward = rows();
    let mut backward = rows();
    backward.reverse();
    order(&mut forward);
    order(&mut backward);
    let paths = |v: &[gitterrace_types::api::PlanFile]| {
        v.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
    };
    assert_eq!(paths(&forward), ["z", "a", "b", "c", "y"]);
    assert_eq!(paths(&forward), paths(&backward));
}

#[test]
fn test_files_and_siblings_are_found_by_the_patterns() {
    let patterns = gitterrace_config::Config::default().review.test_siblings;
    for t in ["src/a_test.rs", "web/a.spec.ts", "tests/x.rs", "pkg/test_a.py", "a/__tests__/b.js"] {
        assert!(is_test(t, &patterns), "{t}");
    }
    for s in ["src/a.rs", "web/a.ts", "README.md"] {
        assert!(!is_test(s, &patterns), "{s}");
    }
    let tree: BTreeSet<&str> = ["src/a.rs", "src/a_test.rs", "web/b.ts", "tests/deep/c.rs"].into();
    assert!(has_sibling("src/a.rs", &patterns, &tree));
    assert!(!has_sibling("web/b.ts", &patterns, &tree));
    assert!(has_sibling("src/c.rs", &patterns, &tree), "tests/**/{{stem}}.* style is by stem");
}

#[test]
fn glob_stops_a_star_at_a_folder_and_a_double_star_does_not() {
    assert!(glob("tests/**/c.*", "tests/c.rs"));
    assert!(glob("tests/**/c.*", "tests/a/b/c.rs"));
    assert!(glob("a/*.rs", "a/x.rs"));
    assert!(!glob("a/*.rs", "a/b/x.rs"));
}

fn state(dir: &std::path::Path) -> ApiState {
    let _ = Fixture::init(dir)
        .write("src/a.rs", "fn a() {}\n")
        .write("src/a_test.rs", "fn t() {}\n")
        .write("src/b.rs", "fn b() {}\n")
        .write("Cargo.lock", "# lock\n")
        .commit("feat: first")
        .write("src/b.rs", "fn b() {}\nfn c() {}\n")
        .commit("fix: second");
    let s = ApiState::open(dir).expect("open");
    s.refresh_index().expect("index");
    s
}

/// The plan over a real working tree: scored files sum their terms, the lockfile is named and
/// unscored, the same request answers the same bytes, and a public handle refuses it.
#[test]
fn a_plan_lists_scored_files_by_risk_and_names_the_skipped() {
    let d = tempfile::tempdir().expect("tmp");
    let s = state(d.path());
    std::fs::write(d.path().join("src/b.rs"), "fn b() {}\nfn c() {}\nfn d() {}\n").expect("w");
    std::fs::write(d.path().join("src/new.rs"), "fn n() {}\n").expect("w");
    std::fs::write(d.path().join("Cargo.lock"), "# lock\n# more\n").expect("w");
    let req = PlanRequest::default();
    let plan = s.review_plan(&req, &Params::default()).expect("plan");
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].path, "Cargo.lock");
    assert_eq!(plan.total_files, 3);
    assert!(!plan.truncated);
    let new = plan.files.iter().find(|f| f.path == "src/new.rs").expect("new");
    assert_eq!(new.terms.untested, 20, "a new source file with no test");
    let b = plan.files.iter().find(|f| f.path == "src/b.rs").expect("b");
    assert_eq!(b.terms.untested, 20);
    for f in &plan.files {
        // The terms are read back from the JSON, so a term that feeds `risk` but is not a
        // named field of `terms` makes this fail.
        let v = serde_json::to_value(f.terms).expect("json");
        let named: u64 =
            v.as_object().expect("object").values().filter_map(serde_json::Value::as_u64).sum();
        assert_eq!(u64::from(f.risk), named, "{}", f.path);
        assert!(f.risk <= 100);
    }
    let again = s.review_plan(&req, &Params::default()).expect("plan");
    assert_eq!(serde_json::to_string(&plan).expect("a"), serde_json::to_string(&again).expect("b"));
    let cut = s.review_plan(&req, &Params { limit: Some(1), ..Params::default() }).expect("cut");
    assert_eq!((cut.files.len(), cut.truncated, cut.total_files), (1, true, 3));
    let public = ApiState::open(d.path()).expect("open").committed_only();
    assert!(matches!(
        public.review_plan(&req, &Params::default()),
        Err(crate::ApiError::Forbidden(_))
    ));
}

#[test]
fn a_compare_plan_needs_a_base_and_scores_the_branch() {
    let d = tempfile::tempdir().expect("tmp");
    let s = state(d.path());
    let none = PlanRequest { source: Some("compare"), ..PlanRequest::default() };
    assert!(matches!(
        s.review_plan(&none, &Params::default()),
        Err(crate::ApiError::BadRequest(_))
    ));
    let bad = PlanRequest { source: Some("nope"), ..PlanRequest::default() };
    assert!(s.review_plan(&bad, &Params::default()).is_err());
    git(d.path(), &["log", "-1"]);
    let req =
        PlanRequest { source: Some("compare"), base: Some("HEAD~1"), ..PlanRequest::default() };
    let plan = s.review_plan(&req, &Params::default()).expect("plan");
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].path, "src/b.rs");
    assert!(plan.head.is_some());
    assert!(plan.files[0].permalink.is_some());
}
