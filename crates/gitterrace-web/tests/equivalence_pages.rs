//! Output equivalence: every insights page (rendered HTML) and every API insight (JSON) under
//! a pairwise covering array of the option matrix plus named high-risk cells, clock frozen, equals
//! today's answer byte for byte. Today's in-memory implementation wrote `tests/golden/*.txt`
//! (`TEST_BLESS=1`); any later implementation must reproduce it. A failure dumps the differing
//! bodies beside the golden.
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
        .map(|(k, v)| (k, millis(&v.replace(&t, "<TMP>"))))
        .collect()
}

/// Blanks the elapsed time a search page prints (`in 8 ms`): wall clock, not output.
fn millis(body: &str) -> String {
    let re = regex::Regex::new(r" in \d+ ms").expect("regex");
    re.replace_all(body, " in _ ms").into_owned()
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

/// Ids an HTML body gives more than once. A repeated id sends a `#fragment`, a label or a
/// script to whichever element comes first.
fn duplicate_ids(body: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut twice = std::collections::BTreeSet::new();
    for part in body.split(" id=\"").skip(1) {
        let id = part.split('"').next().unwrap_or_default();
        if !seen.insert(id) {
            twice.insert(id.to_owned());
        }
    }
    twice.into_iter().collect()
}

#[test]
fn no_page_gives_an_id_twice() {
    assert_eq!(
        duplicate_ids(r#"<a id="x"></a><b id="y"></b><i id="x"></i>"#),
        ["x"]
    );
    if !oracle::frozen("no_page_gives_an_id_twice") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let bad: Vec<String> = rt
        .block_on(golden_answers())
        .iter()
        .filter_map(|(key, body)| {
            let d = duplicate_ids(body);
            (!d.is_empty()).then(|| format!("{key}: {d:?}"))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "pages with a repeated id:\n{}",
        bad.join("\n")
    );
}

/// The per-commit proof that the pairwise golden loses nothing: at this build, run the full
/// product and the golden's cell set, then compare every shared key. Writes counts and the
/// differences, classified chrome-only, tie-only or lost, to `$TEST_GOLDEN_PROOF`, and fails
/// when any is lost. It never compares two commits, so it says nothing about a re-bless: that is
/// `a_rebless_proof_classifies_old_against_new`. With `TEST_GOLDEN_FULL=<old full golden>` the
/// full run is also compared to it by digest. Off unless `TEST_GOLDEN_PROOF` is set.
#[test]
fn the_pairwise_golden_agrees_with_the_full_matrix() {
    let Some(out) = std::env::var_os("TEST_GOLDEN_PROOF") else {
        return;
    };
    if !oracle::frozen("the_pairwise_golden_agrees_with_the_full_matrix") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let all = matrix::cells();
    let mut full = BTreeMap::new();
    for mm in [Mailmap::AsIs, Mailmap::Edited, Mailmap::CaseFold] {
        let cells: Vec<_> = all.iter().copied().filter(|c| c.mailmap == mm).collect();
        full.extend(rt.block_on(run(mm, &cells)));
    }
    let small = rt.block_on(golden_answers());
    let mut report = oracle::Proof::default();
    for (k, v) in &small {
        if let Some(w) = full.get(k) {
            report.add(k, w, v);
        } else {
            report.only_small += 1;
        }
    }
    report.full = full.len();
    report.small = small.len();
    if let Some(old) = std::env::var_os("TEST_GOLDEN_FULL") {
        report.against_golden(&std::fs::read_to_string(old).expect("full golden"), &full);
    }
    let text = report.render(&format!(
        "pairwise cells {}, named cells {}",
        matrix::pairwise().len(),
        matrix::named().len()
    ));
    std::fs::write(&out, &text).expect("proof file");
    assert_eq!(report.real.len(), 0, "{text}");
}

/// The browsing pages, one or more cells each, over the matrix repository at HEAD. Every page
/// kind the router serves has a cell here or on an insights page, or is exempt below.
const BROWSE: [&str; 33] = [
    "/log",
    "/log/HEAD",
    "/log/HEAD?path=src",
    "/history/src/lib.rs",
    "/compare",
    "/compare?base=HEAD~3&head=HEAD",
    "/commit/HEAD",
    "/commit/HEAD~1",
    "/diff/HEAD/0",
    "/tree/HEAD",
    "/tree/HEAD/",
    "/tree/HEAD/src",
    "/tree/HEAD/docs",
    "/blob/HEAD/src/lib.rs",
    "/blob/HEAD/docs/x.md",
    "/blob/HEAD/vendor/x.c",
    "/hl/HEAD/src/lib.rs",
    "/raw/HEAD/src/lib.rs",
    "/blame/HEAD/src/lib.rs",
    "/refs",
    "/tag/v1",
    "/worktrees",
    "/stashes",
    "/worktree",
    "/worktree/diff/src/lib.rs",
    "/search?q=fn",
    "/search?q=feat&kind=commits",
    "/search?q=nothing-matches-this",
    "/stats/types",
    "/stats/frequency",
    "/stats/patterns",
    "/api/refs",
    "/api/files/HEAD",
];

/// Routes with no golden cell, each with the reason. A route listed here that does not exist
/// fails, so the list can only follow the router.
const EXEMPT: [(&str, &str); 17] = [
    (
        "/api/last/{rev}",
        "a fragment the tree page fetches, filled by the file walk",
    ),
    ("/api/last/{rev}/{*path}", "same fragment, below the root"),
    ("/api/progress", "indexing state, changes with timing"),
    ("/events", "an event stream, never a page"),
    ("/go/{*rest}", "a redirect to a served repository by id"),
    ("/_send/targets", "targets from the user's actions file"),
    (
        "/r/{name}/_send/targets",
        "same, under the repository prefix",
    ),
    ("/r/{name}", "forwards to the repository"),
    ("/r/{name}/", "forwards to the repository"),
    (
        "/r/{name}/{*rest}",
        "the repository prefix, covered by every cell",
    ),
    (
        "/api/repos",
        "the server's repository list, holds a temp path",
    ),
    (
        "/_repos",
        "adds and removes repositories, secret and loopback only",
    ),
    ("/_settings", "reads the user's config file"),
    ("/_settings/file", "reads and writes the user's config file"),
    (
        "/_settings/check",
        "checks a posted config file, never a page",
    ),
    ("/insights", "a permanent redirect to /stats"),
    (
        "/worktree/raw/{*path}",
        "image bytes of a changed file, none in the clean matrix repo",
    ),
];

/// Whether the route literal `route` (`/blob/{rev}/{*path}`) serves the concrete `path`.
fn serves(route: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or_default();
    let (r, p): (Vec<&str>, Vec<&str>) = (route.split('/').collect(), path.split('/').collect());
    for (i, seg) in r.iter().enumerate() {
        if seg.starts_with("{*") {
            return p.len() > i && !p[i].is_empty();
        }
        let ok = p.get(i).is_some_and(|q| {
            if seg.starts_with('{') {
                !q.is_empty()
            } else {
                seg == q
            }
        });
        if !ok {
            return false;
        }
    }
    r.len() == p.len()
}

/// Every page kind the registry lists has a cell: an insights page of the matrix, a browsing
/// page in [`BROWSE`], or an entry in [`EXEMPT`] that says why not. Red when a route is added
/// with none of the three.
#[test]
fn every_routed_page_kind_has_a_golden_cell() {
    assert!(serves("/blob/{rev}/{*path}", "/blob/HEAD/a/b.rs"));
    assert!(!serves("/blob/{rev}/{*path}", "/blob/HEAD"));
    assert!(serves("/tree/{rev}/", "/tree/HEAD/") && !serves("/tree/{rev}/", "/tree/HEAD"));
    let routes: Vec<&str> = gitterrace_web::sitemap::PAGES
        .iter()
        .flat_map(|p| p.routes.iter().copied())
        .collect();
    let cells: Vec<&str> = gitterrace_web::WARM_PAGES
        .iter()
        .chain(&BROWSE)
        .copied()
        .collect();
    let bare: Vec<&str> = routes
        .iter()
        .copied()
        .filter(|r| !EXEMPT.iter().any(|(e, _)| e == r))
        .filter(|r| !cells.iter().any(|c| serves(r, c)))
        .collect();
    assert!(bare.is_empty(), "routes with no golden cell: {bare:?}");
    let stale: Vec<&str> = EXEMPT
        .iter()
        .map(|e| e.0)
        .filter(|e| !routes.contains(e))
        .collect();
    assert!(
        stale.is_empty(),
        "EXEMPT names routes that do not exist: {stale:?}"
    );
    let orphans: Vec<&str> = BROWSE
        .iter()
        .copied()
        .filter(|c| !routes.iter().any(|r| serves(r, c)))
        .collect();
    assert!(orphans.is_empty(), "cells no route serves: {orphans:?}");
}

/// The browsing pages against `tests/golden/pages.txt`: one repository, HEAD, frozen clock.
#[test]
fn browsing_pages_match_the_oracle() {
    if !oracle::frozen("browsing_pages_match_the_oracle") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let answers = rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let (repo, _) = oracle::matrix_repo(dir.path(), Mailmap::AsIs);
        let f = gitterrace_testkit::Fixture::init(&repo);
        f.git(&["tag", "v1"]);
        let router = gitterrace_web::app_router(&repo).expect("router");
        oracle::ready(&router).await;
        let mut got = BTreeMap::new();
        for uri in BROWSE {
            let (status, body) = gitterrace_testkit::page(&router, uri).await;
            let body = oracle::normalize(&body);
            let v = if status == 200 {
                body
            } else {
                format!("status {status}\n{body}")
            };
            got.insert(format!("[browse] {uri}"), v);
        }
        scrub(got, dir.path())
    });
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/pages.txt");
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

/// The frozen clock is the only time input: the whole golden matrix, rendered under two local
/// zones on opposite sides of the date line (14 hours east and 10 hours west of UTC), is byte
/// identical. Red if any page reads the process zone, a local "today", or a clock the oracle
/// does not freeze.
#[test]
fn the_matrix_does_not_depend_on_the_local_zone() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("zone-guard");
    let exe = std::env::current_exe().expect("test binary");
    let run = |zone: &str| {
        let out = base.join(zone.replace('/', "-"));
        if out.exists() {
            std::fs::remove_dir_all(&out).expect("remove old dump");
        }
        let status = std::process::Command::new(&exe)
            .args([
                "every_cell_matches_the_oracle",
                "--exact",
                "--test-threads=1",
            ])
            .env(oracle::NOW_VAR, oracle::FROZEN_NOW.to_string())
            .env("TZ", zone)
            .env("TEST_ORACLE_DUMP", &out)
            .env_remove("TEST_BLESS")
            .status()
            .expect("run child");
        assert!(status.success(), "the golden matrix fails under TZ={zone}");
        out
    };
    let (east, west) = (run("Pacific/Kiritimati"), run("America/Adak"));
    let mut names: Vec<_> = std::fs::read_dir(&east)
        .expect("east dump")
        .map(|e| e.expect("entry").file_name())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "the oracle dumped no answers");
    let moved: Vec<_> = names
        .iter()
        .filter(|n| std::fs::read(east.join(n)).ok() != std::fs::read(west.join(n)).ok())
        .collect();
    assert!(
        moved.is_empty(),
        "{} answers depend on the local zone, first {:?}",
        moved.len(),
        moved.first()
    );
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

/// The re-bless proof: two `TEST_ORACLE_DUMP` folders of the same cells, one per commit, compared
/// per cell and classified unchanged, chrome-only, tie-only or real, each real cell with its
/// first differing line. Writes `$TEST_CHANGE_REPORT` and fails on any real or one-sided cell.
/// `scripts/golden-check.sh change OLD NEW` makes the dumps and runs it:
/// `TEST_CHANGE_OLD=<dump> TEST_CHANGE_NEW=<dump> TEST_CHANGE_REPORT=<file>`.
#[test]
fn a_rebless_proof_classifies_old_against_new() {
    let (Some(old), Some(new), Some(out)) = (
        std::env::var_os("TEST_CHANGE_OLD"),
        std::env::var_os("TEST_CHANGE_NEW"),
        std::env::var_os("TEST_CHANGE_REPORT"),
    ) else {
        return;
    };
    let (old, new) = (Path::new(&old), Path::new(&new));
    let change = oracle::Change::of(&oracle::read_dump(old), &oracle::read_dump(new));
    let head = std::env::var("TEST_CHANGE_HEAD").unwrap_or_default();
    let text = change.render(&head);
    std::fs::write(&out, &text).expect("change report");
    assert_eq!(change.unexplained(), 0, "{text}");
}
