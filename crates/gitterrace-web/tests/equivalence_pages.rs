//! Output equivalence: every insights page (rendered HTML) and every API insight (JSON) under
//! a pairwise covering array of the option matrix plus named high-risk cells, clock frozen, equals today's answer byte for byte.
//! Today's in-memory implementation wrote `tests/golden/*.txt` (`TEST_BLESS=1`); any later
//! implementation must reproduce it. A failure dumps the differing bodies beside the golden.
//!
//! `TEST_GITGIT=/path/to/git` adds a seeded sample of 64 cells over a real history. Its golden is
//! kept outside the repository, at `$TEST_GITGIT_GOLDEN` (default `target/gitgit-golden.txt`).
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::collections::BTreeMap;
use std::path::Path;

use gitterrace_testkit::matrix::{self, Mailmap};
use gitterrace_testkit::oracle;

/// Where a failing run writes the differing bodies: under `target/`, never the source tree.
fn dump(golden: &Path) -> std::path::PathBuf {
    let name = golden
        .file_stem()
        .map_or_else(|| "golden".into(), |s| s.to_string_lossy());
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.actual"))
}

fn targets() -> Vec<String> {
    let names: Vec<&str> = gitterrace_api::insight_list().map(|(n, _)| n).collect();
    oracle::targets(&gitterrace_web::WARM_PAGES, &names)
}

/// The temp folder differs per run: it must not reach a digest.
fn scrub(answers: BTreeMap<String, String>, tmp: &Path) -> BTreeMap<String, String> {
    let t = tmp.to_string_lossy().into_owned();
    answers
        .into_iter()
        .map(|(k, v)| (k, v.replace(&t, "<TMP>")))
        .collect()
}

const fn tag(mm: Mailmap) -> &'static str {
    match mm {
        Mailmap::AsIs => "as-is",
        Mailmap::Edited => "edited",
        Mailmap::CaseFold => "case-fold",
    }
}

async fn run(mm: Mailmap, cells: &[matrix::Cell]) -> BTreeMap<String, String> {
    let dir = tempfile::tempdir().expect("tmp");
    let (repo, values) = oracle::matrix_repo(dir.path(), mm);
    let router = gitterrace_web::app_router(&repo).expect("router");
    oracle::ready(&router).await;
    let tag = tag(mm);
    let got = oracle::capture(&router, &targets(), cells, &values).await;
    scrub(got, dir.path())
        .into_iter()
        .map(|(k, v)| (format!("[{tag}] {k}"), v))
        .collect()
}

/// A renamed-and-reused path: `src/lib.rs` moves to `src/core.rs`, and a new `src/lib.rs`
/// starts over. Ownership over all time must not merge the two histories.
fn renamed_repo(dir: &Path) -> (std::path::PathBuf, matrix::Values) {
    let (repo, values) = oracle::matrix_repo(dir, Mailmap::AsIs);
    let f = gitterrace_testkit::Fixture::init(&repo);
    f.git(&["mv", "src/lib.rs", "src/core.rs"]);
    let f = f
        .at("2024-03-09T11:00:00Z")
        .commit("refactor(core): move lib to core")
        .at("2024-03-09T12:00:00Z")
        .write("src/lib.rs", "pub mod core;\n")
        .commit("feat(core): a new lib");
    (f.path().to_owned(), values)
}

/// The equivalence golden's cell set: the pairwise array on every target, plus the named
/// high-risk cells on their pages. Keyed `[repo] target?query`.
async fn golden_answers() -> BTreeMap<String, String> {
    let set = matrix::pairwise();
    let named = matrix::named();
    let mut answers = BTreeMap::new();
    for mm in [Mailmap::AsIs, Mailmap::Edited, Mailmap::CaseFold] {
        let cells: Vec<_> = set.iter().copied().filter(|c| c.mailmap == mm).collect();
        answers.extend(run(mm, &cells).await);
        let dir = tempfile::tempdir().expect("tmp");
        let (repo, values) = oracle::matrix_repo(dir.path(), mm);
        let router = gitterrace_web::app_router(&repo).expect("router");
        oracle::ready(&router).await;
        for n in named.iter().filter(|n| n.cell.mailmap == mm) {
            let got = oracle::capture(&router, &[n.page.to_owned()], &[n.cell], &values).await;
            answers.extend(
                scrub(got, dir.path())
                    .into_iter()
                    .map(|(k, v)| (format!("[{}] {k}", tag(mm)), v)),
            );
        }
    }
    let dir = tempfile::tempdir().expect("tmp");
    let (repo, values) = renamed_repo(dir.path());
    let router = gitterrace_web::app_router(&repo).expect("router");
    oracle::ready(&router).await;
    let cells: Vec<_> = named
        .iter()
        .filter(|n| n.page.contains("ownership") && n.cell.mailmap == Mailmap::AsIs)
        .map(|n| n.cell)
        .collect();
    let pages = [
        "/stats/ownership".to_owned(),
        "/api/v1/insights/ownership".to_owned(),
    ];
    let got = oracle::capture(&router, &pages, &cells, &values).await;
    answers.extend(
        scrub(got, dir.path())
            .into_iter()
            .map(|(k, v)| (format!("[renamed] {k}"), v)),
    );
    answers
}

/// Pairwise plus named cells, against `tests/golden/equivalence.txt`.
#[test]
fn every_cell_matches_the_oracle() {
    if !oracle::frozen("every_cell_matches_the_oracle") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let mut answers = rt.block_on(golden_answers());
    if let Ok(key) = std::env::var("TEST_ORACLE_PLANT") {
        // The red proof: one byte of one body changed must fail the check.
        let body = answers.get_mut(&key).expect("planted key is a golden key");
        body.push('x');
    }
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/equivalence.txt");
    oracle::check(&golden, &dump(&golden), &answers);
}

/// The pages and API for a revision other than HEAD: the tip of `topic`, a branch with commits
/// HEAD lacks, and `HEAD~3`, an older ancestor. Selected by `?rev=`, as the pages link it. Each
/// rev must answer from its own history: serving HEAD's set for either one is red.
#[test]
fn other_revisions_match_the_oracle() {
    if !oracle::frozen("other_revisions_match_the_oracle") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let answers = rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let (repo, values) = oracle::matrix_repo(dir.path(), Mailmap::AsIs);
        let f = gitterrace_testkit::Fixture::init(&repo)
            .checkout("HEAD~3")
            .branch("topic")
            .at("2024-03-07T09:00:00Z")
            .write("topic/a.rs", "fn t() {}\n")
            .commit("feat(topic): only on the branch")
            .at("2024-03-07T11:00:00Z")
            .write("src/lib.rs", "fn topic() {}\n")
            .commit("fix(core): topic edit")
            .checkout("main");
        let router = gitterrace_web::app_router(f.path()).expect("router");
        oracle::ready(&router).await;
        let cells: Vec<_> = matrix::cells()
            .into_iter()
            .filter(|c| c.mailmap == Mailmap::AsIs && !c.ws && !c.bots)
            .filter(|c| c.filter == matrix::PathFilter::None)
            .collect();
        let mut out = BTreeMap::new();
        for rev in ["topic", "HEAD~3"] {
            oracle::ready_at(&router, rev).await;
            let got = oracle::capture_rev(&router, &targets(), &cells, &values, rev).await;
            out.extend(
                scrub(got, dir.path())
                    .into_iter()
                    .map(|(k, v)| (format!("[{rev}] {k}"), v)),
            );
        }
        out
    });
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/revisions.txt");
    oracle::check(&golden, &dump(&golden), &answers);
}

/// The frozen clock really reaches the pages: without it the 1w window of the fixture is empty,
/// with it the week's commit shows. Red if any of the hooks reads the wall clock again.
#[test]
fn the_frozen_clock_reaches_every_page() {
    if !oracle::frozen("the_frozen_clock_reaches_every_page") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let (repo, _) = oracle::matrix_repo(dir.path(), Mailmap::AsIs);
        let router = gitterrace_web::app_router(&repo).expect("router");
        oracle::ready(&router).await;
        let (_, api) =
            gitterrace_testkit::page(&router, "/api/v1/insights/summary?period=1w").await;
        let v: serde_json::Value = serde_json::from_str(&api).expect("json");
        // The newest commit not after the frozen now is 2024-03-08 or 03-09: the reference
        // time is that, not the wall clock's 2026.
        assert!(
            v.to_string().contains("1709") || v.to_string().contains("1710"),
            "{v}"
        );
        // Relative dates count from the frozen now (web fmt::now): a commit two days before it
        // reads "2 days ago", against the wall clock it would read years.
        let plain = dir.path().join("plain");
        let plain = gitterrace_testkit::Fixture::init(&plain)
            .at("2024-03-08T00:00:00Z")
            .write("a.txt", "a\n")
            .commit("feat: a");
        let router = gitterrace_web::app_router(plain.path()).expect("router");
        let (_, log) = gitterrace_testkit::page(&router, "/log/HEAD").await;
        assert!(log.contains("2 days ago"), "web clock not frozen");
    });
}

#[test]
fn a_real_history_samples_match_the_oracle() {
    let Some(repo) = std::env::var_os("TEST_GITGIT") else {
        return;
    };
    if !oracle::frozen("a_real_history_samples_match_the_oracle") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let values = matrix::Values {
        author: "gitster@pobox.com".to_owned(),
        kind: "fix".to_owned(),
        scope: "t".to_owned(),
        path: "builtin".to_owned(),
        exclude: "t/**".to_owned(),
    };
    let cells: Vec<_> = matrix::sample(64, 64)
        .into_iter()
        .filter(|c| c.mailmap == Mailmap::AsIs)
        .collect();
    let answers = rt.block_on(async {
        let router = gitterrace_web::app_router(Path::new(&repo)).expect("router");
        oracle::ready(&router).await;
        oracle::capture(&router, &targets(), &cells, &values).await
    });
    let golden = std::env::var_os("TEST_GITGIT_GOLDEN").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/gitgit-golden.txt"),
        Into::into,
    );
    oracle::check(&golden, &dump(&golden), &answers);
}
