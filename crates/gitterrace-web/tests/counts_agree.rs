//! Every count agrees with its parts and with every other place that shows it.
//!
//! Over the pairwise option matrix: the commits of a period equal the sum of its bars, of its
//! weekday and hour split, of its type breakdown and of its contributors; its lines equal the
//! sum of the frequency buckets and of the contributors' lines. Each insights page shows the
//! number the API gives for the same query. A page that prints a count next to a list (file
//! history, commit search, the contributors list, the overview over the log) prints the length
//! of that list.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::matrix::{self, Mailmap};
use gitterrace_testkit::{Fixture, oracle, page};
use serde_json::Value;

async fn json(router: &axum::Router, uri: &str) -> Value {
    let (status, body) = page(router, uri).await;
    assert_eq!(status, 200, "{uri}: {body}");
    serde_json::from_str::<Value>(&body).expect("json")
}

async fn data(router: &axum::Router, uri: &str) -> Value {
    json(router, uri).await["data"].clone()
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap_or_else(|| panic!("not a count: {v}"))
}

fn sum(v: &Value, f: impl Fn(&Value) -> u64) -> u64 {
    v.as_array().expect("list").iter().map(f).sum()
}

/// A number as a page writes it: thousands commas and a leading sign dropped.
fn parse(s: &str) -> u64 {
    s.trim()
        .trim_start_matches(['+', '-'])
        .replace(',', "")
        .parse()
        .unwrap_or_else(|_| panic!("not a number: {s:?}"))
}

/// The value of the KPI tile labelled `label`: `<b>V</b><span>label</span>`.
fn kpi(body: &str, label: &str) -> Option<u64> {
    let at = body.find(&format!("</b><span>{label}</span>"))?;
    let open = body[..at].rfind('>')?;
    Some(parse(&body[open + 1..at]))
}

/// The "This period" cell of the activity table row labelled `label`.
fn pulse(body: &str, label: &str) -> Option<u64> {
    let at = body.find(&format!(">{label}</th><td class=\"r\"><b>"))?;
    let rest = &body[at + label.len() + 1..];
    let start = rest.find("<b>")? + 3;
    let end = rest[start..].find("</b>")? + start;
    Some(parse(&rest[start..end]))
}

/// Commit rows a list page renders.
fn rows(body: &str) -> usize {
    body.matches("<li class=\"crow\"").count()
}

/// The number written right before `noun` in the first place `noun` follows a number.
fn count_before(body: &str, noun: &str) -> Option<u64> {
    let mut from = 0;
    while let Some(i) = body[from..].find(noun) {
        let at = from + i;
        let head = body[..at].trim_end().trim_end_matches("</b>").trim_end();
        let digits: String = head
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == ',')
            .collect();
        if !digits.is_empty() {
            return Some(parse(&digits.chars().rev().collect::<String>()));
        }
        from = at + noun.len();
    }
    None
}

/// The parts of one query's numbers, read from the API.
async fn parts_agree(router: &axum::Router, q: &str, fails: &mut Vec<String>) {
    let api = "/api/v1/insights";
    let s = data(router, &format!("{api}/summary?{q}")).await;
    let cur = &s["current"];
    let (commits, add, del) = (
        n(&cur["commits"]),
        n(&cur["additions"]),
        n(&cur["deletions"]),
    );
    let r = data(router, &format!("{api}/activity?{q}")).await;
    let lb = data(router, &format!("{api}/label_breakdown?{q}")).await;
    let co = data(router, &format!("{api}/contributors?{q}")).await;
    let fr = data(router, &format!("{api}/frequency?{q}")).await;
    let people = &co["contributors"];
    let mut check = |what: &str, got: u64, want: u64| {
        if got != want {
            fails.push(format!("{q}: {what} {got}, summary says {want}"));
        }
    };
    check(
        "commits over the bars",
        sum(&r["series"], |p| n(&p["commits"])),
        commits,
    );
    check(
        "commits over the weekdays",
        sum(&r["by_weekday"], n),
        commits,
    );
    check("commits over the hours", sum(&r["by_hour"], n), commits);
    check(
        "commits over the punch card",
        sum(&r["punch_card"], |d| sum(d, n)),
        commits,
    );
    check(
        "commits over the types",
        sum(&lb, |l| n(&l["commits"])),
        commits,
    );
    check(
        "commits over the contributors",
        sum(people, |p| n(&p["commits"])),
        commits,
    );
    check(
        "lines added over the bars",
        sum(&r["series"], |p| n(&p["additions"])),
        add,
    );
    check(
        "lines added over the frequency",
        sum(&fr, |w| n(&w["additions"])),
        add,
    );
    check(
        "lines removed over the frequency",
        sum(&fr, |w| n(&w["deletions"])),
        del,
    );
    check(
        "lines added over the contributors",
        sum(people, |p| n(&p["additions"])),
        add,
    );
    check(
        "lines removed over the contributors",
        sum(people, |p| n(&p["deletions"])),
        del,
    );
    check(
        "authors over the contributors",
        people.as_array().map_or(0, Vec::len) as u64,
        n(&cur["authors"]),
    );
}

/// Each insights page shows what the API says for the same query.
async fn pages_agree(router: &axum::Router, q: &str, fails: &mut Vec<String>) {
    let api = "/api/v1/insights";
    let mut check = |what: &str, got: Option<u64>, want: u64| {
        if got != Some(want) {
            fails.push(format!("{q}: {what} shows {got:?}, the API says {want}"));
        }
    };
    let s = data(router, &format!("{api}/summary?{q}")).await;
    let cur = &s["current"];
    let (_, stats) = page(router, &format!("/stats?{q}")).await;
    if n(&cur["commits"]) > 0 {
        check(
            "/stats Commits",
            pulse(&stats, "Commits without merges"),
            n(&cur["commits"]),
        );
        check(
            "/stats Authors",
            pulse(&stats, "Authors"),
            n(&cur["authors"]),
        );
        check(
            "/stats Files changed",
            pulse(&stats, "Files changed"),
            n(&cur["files"]),
        );
        check(
            "/stats Lines added",
            pulse(&stats, "Lines added"),
            n(&cur["additions"]),
        );
        check(
            "/stats Lines removed",
            pulse(&stats, "Lines removed"),
            n(&cur["deletions"]),
        );
        let fr = data(router, &format!("{api}/frequency?{q}")).await;
        check(
            "frequency Lines added",
            kpi(&stats, "Lines added"),
            sum(&fr, |w| n(&w["additions"])),
        );
    }
    let co = data(router, &format!("{api}/contributors?{q}")).await;
    let people = co["contributors"].as_array().expect("people");
    let (_, cp) = page(router, &format!("/stats/contributors?{q}&all=1")).await;
    if !people.is_empty() {
        check(
            "contributors KPI",
            kpi(&cp, "Contributors"),
            people.len() as u64,
        );
        check(
            "contributors New in period",
            kpi(&cp, "New in period"),
            people.iter().filter(|p| p["first_time"] == true).count() as u64,
        );
        check(
            "contributors rows",
            Some(cp.matches("<a class=\"who\"").count() as u64),
            people.len() as u64,
        );
    }
    let ho = data(router, &format!("{api}/hotspots?{q}&limit=100000")).await;
    let (_, hp) = page(router, &format!("/stats/hotspots?{q}")).await;
    if n(&ho["total_changes"]) > 0 {
        check(
            "hotspots Changes",
            kpi(&hp, "Changes"),
            n(&ho["total_changes"]),
        );
        check(
            "hotspots Files changed",
            kpi(&hp, "Files changed"),
            ho["files"].as_array().map_or(0, Vec::len) as u64,
        );
    }
    let ow = data(router, &format!("{api}/ownership?{q}")).await;
    let (_, op) = page(router, &format!("/stats/ownership?{q}")).await;
    if !ow["dirs"].as_array().is_none_or(Vec::is_empty) {
        check(
            "ownership Bus factor",
            kpi(&op, "Bus factor"),
            n(&ow["overall"]["bus_factor"]),
        );
        check(
            "ownership Solo lines",
            kpi(&op, "Solo lines"),
            n(&ow["solo_lines"]),
        );
        check(
            "ownership Stale lines",
            kpi(&op, "Stale lines"),
            n(&ow["stale_lines"]),
        );
    }
}

/// `/contributors` and `/insights/contributors` are one figure: under the same query they list
/// the same people with the same commits.
async fn contributors_agree(router: &axum::Router, q: &str, fails: &mut Vec<String>) {
    let plain = json(router, &format!("/api/v1/contributors?{q}")).await;
    let insight = data(router, &format!("/api/v1/insights/contributors?{q}")).await;
    let pick = |v: &Value, key: &str| -> Vec<(String, u64)> {
        v[key]
            .as_array()
            .expect("people")
            .iter()
            .map(|p| {
                (
                    p["name"].as_str().unwrap_or_default().to_owned(),
                    n(&p["commits"]),
                )
            })
            .collect()
    };
    let (a, b) = (pick(&plain, "authors"), pick(&insight, "contributors"));
    if a != b {
        fails.push(format!(
            "{q}: /contributors {a:?}, /insights/contributors {b:?}"
        ));
    }
}

/// Every pairwise cell of every mailmap variant of the matrix repository.
async fn each_cell(mut f: impl AsyncFnMut(&axum::Router, String)) {
    for mm in [Mailmap::AsIs, Mailmap::Edited, Mailmap::CaseFold] {
        let dir = tempfile::tempdir().expect("tmp");
        let (repo, values) = oracle::matrix_repo(dir.path(), mm);
        let router = gitterrace_web::app_router(&repo).expect("router");
        oracle::ready(&router).await;
        for cell in matrix::pairwise().iter().filter(|c| c.mailmap == mm) {
            f(&router, cell.query(&values)).await;
        }
    }
}

#[tokio::test]
async fn counts_equal_the_sum_of_their_parts() {
    let mut fails = Vec::new();
    each_cell(async |r, q| parts_agree(r, &q, &mut fails).await).await;
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// Every page shows the API's numbers, and both contributor endpoints agree, over one build
/// of each fixture.
#[tokio::test]
async fn every_page_shows_the_api_numbers() {
    let mut fails = Vec::new();
    each_cell(async |r, q| {
        pages_agree(r, &q, &mut fails).await;
        contributors_agree(r, &q, &mut fails).await;
    })
    .await;
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// Ann commits three times, a branch is merged, `dependabot[bot]` commits twice.
fn mixed(dir: &std::path::Path) -> std::path::PathBuf {
    let f = Fixture::init(dir).at("2024-03-01T10:00:00Z");
    f.git(&["config", "user.name", "Ann"]);
    f.git(&["config", "user.email", "ann@x.io"]);
    let f = f
        .write("src/a.rs", "a\n")
        .commit("feat: a")
        .branch("side")
        .write("src/b.rs", "b\n")
        .commit("feat: b")
        .checkout("main")
        .write("src/c.rs", "c\n")
        .commit("feat: c");
    f.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    f.git(&["config", "user.name", "dependabot[bot]"]);
    f.git(&[
        "config",
        "user.email",
        "49699333+dependabot[bot]@users.noreply.github.com",
    ]);
    let f = f
        .write("Cargo.toml", "1\n")
        .commit("chore(deps): bump")
        .write("Cargo.toml", "2\n")
        .commit("chore(deps): bump");
    f.path().to_owned()
}

/// The overview's commits are every commit, and the log lists exactly those.
#[tokio::test]
async fn the_overview_counts_the_log() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = gitterrace_web::app_router(mixed(dir.path())).expect("router");
    oracle::ready(&router).await;
    let (_, ov) = page(&router, "/").await;
    let shown = count_before(&ov, " commits").expect("overview commits");
    let (mut listed, mut pg) = (0, 0);
    loop {
        let (_, log) = page(&router, &format!("/log/HEAD?page={pg}")).await;
        listed += rows(&log);
        if !log.contains(">Older</a>") {
            break;
        }
        pg += 1;
    }
    assert_eq!(shown, listed as u64, "{ov}");
}

/// The overview and `/repo` describe the same history with the same numbers: every commit
/// next to every commit, the counted ones next to the counted ones. Ann's three commits, one
/// merge and two bot commits: six in all, three counted.
#[tokio::test]
async fn the_overview_and_the_api_agree() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = gitterrace_web::app_router(mixed(dir.path())).expect("router");
    oracle::ready(&router).await;
    let (_, ov) = page(&router, "/").await;
    let repo = json(&router, "/api/v1/repo").await;
    let t = &repo["totals"];
    assert_eq!((n(&t["every_commits"]), n(&t["commits"])), (6, 3), "{repo}");
    assert_eq!(count_before(&ov, " author"), Some(n(&t["authors"])), "{ov}");
    assert_eq!(
        count_before(&ov, " commits"),
        Some(n(&t["every_commits"])),
        "{ov}"
    );
    assert_eq!(
        count_before(&ov, " counted"),
        Some(n(&t["commits"])),
        "{ov} {repo}"
    );
    assert_eq!(count_before(&ov, " by bots"), Some(2), "{ov}");
}

/// A file's history says how many commits it lists, and the commit search how many it found.
#[tokio::test]
async fn a_count_beside_a_list_is_its_length() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = gitterrace_web::app_router(mixed(dir.path())).expect("router");
    oracle::ready(&router).await;
    for p in ["src/a.rs", "Cargo.toml", "src/b.rs"] {
        let (_, h) = page(&router, &format!("/history/{p}")).await;
        let said = count_before(&h, " commit").expect("history count");
        assert_eq!(said, rows(&h) as u64, "{p}: {h}");
    }
    for q in ["feat", "bump", "merge"] {
        let (_, s) = page(&router, &format!("/search?q={q}&kind=commits")).await;
        let said = count_before(&s, " commit").expect("search count");
        assert_eq!(said, rows(&s) as u64, "{q}: {s}");
    }
}

/// A file's history, its hotspots row and the sidebar's hot files count the same commits:
/// with bots left out, the two dependabot bumps of `Cargo.toml` count nowhere.
#[tokio::test]
async fn file_counts_follow_the_bot_option() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = gitterrace_web::app_router(mixed(dir.path())).expect("router");
    oracle::ready(&router).await;
    let (_, h) = page(&router, "/history/Cargo.toml").await;
    assert_eq!(rows(&h), 0, "{h}");
    let (_, ov) = page(&router, "/").await;
    assert!(!ov.contains("data-tip=\"Cargo.toml\""), "{ov}");
    let (_, h) = page(&router, "/history/src/a.rs").await;
    assert_eq!(rows(&h), 1, "{h}");
}

/// Every number on the overview's facts line names the commits it counts, and a line that
/// mixes every commit with the counted ones labels both.
#[tokio::test]
async fn every_overview_number_names_its_population() {
    let dir = tempfile::tempdir().expect("tmp");
    let router = gitterrace_web::app_router(mixed(dir.path())).expect("router");
    oracle::ready(&router).await;
    let (_, ov) = page(&router, "/").await;
    let at = ov.find("<p class=\"facts\">").expect("facts line");
    let line = &ov[at..at + ov[at..].find("</p>").expect("end")];
    let spans = line.matches("<span").count();
    assert!(spans >= 5, "{line}");
    assert_eq!(
        line.matches("<span data-population=\"").count()
            + line
                .matches("<span class=\"muted\" data-population=\"")
                .count(),
        spans,
        "{line}"
    );
    assert!(
        line.starts_with("<p class=\"facts\"><span data-population=\"every\"><b>6</b>"),
        "{line}"
    );
    assert!(
        line.contains("data-population=\"counted\"><b>1</b> author"),
        "{line}"
    );
}
