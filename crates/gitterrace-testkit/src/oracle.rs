//! The equivalence oracle: today's answers of every insights page and API, captured over the
//! generated option matrix with the clock frozen, kept as one golden digest per answer.
//!
//! The golden file is the reference. A later implementation passes when every digest is the
//! same. `TEST_BLESS=1` rewrites the file, which only today's implementation may do.

mod change;
mod proof;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use axum::Router;

use crate::matrix::{Cell, Mailmap, Values};
use crate::web::page;

pub use change::{Change, read_dump};
pub use proof::Proof;

/// The frozen "now": 2024-03-10T00:00:00Z, nine days after the adversarial tip.
pub const FROZEN_NOW: i64 = 1_710_028_800;

/// The variable every clock in the server reads before the wall clock.
pub const NOW_VAR: &str = "GITTERRACE_TEST_NOW";

/// Runs the calling test in a child process with the clock frozen, since a test may not set its
/// own environment (`set_var` is unsafe and the workspace forbids unsafe code).
///
/// Returns `true` inside the child, where the test body runs. Returns `false` in the parent
/// after the child passed, panics when it failed. `name` is the test's path as libtest prints
/// it, for example `cells_match_the_golden_digests`.
#[must_use]
pub fn frozen(name: &str) -> bool {
    if std::env::var(NOW_VAR).is_ok_and(|v| v == FROZEN_NOW.to_string()) {
        return true;
    }
    let exe = std::env::current_exe().expect("test binary");
    let status = std::process::Command::new(exe)
        .args([
            name,
            "--exact",
            "--nocapture",
            "--test-threads=1",
            "--include-ignored",
        ])
        .env(NOW_VAR, FROZEN_NOW.to_string())
        .status()
        .expect("run child");
    assert!(status.success(), "{name} failed with the clock frozen");
    false
}

/// The adversarial history plus one commit per option axis, built at `dir/repo`.
///
/// A bot, a lock file (generated), a tracked file `.gitignore` rules out, a whitespace-only change,
/// a vendored directory (the exclude glob) and dates inside each period window before
/// [`FROZEN_NOW`].
/// `Edited` adds a last commit that rewrites `.mailmap` to rename Bob. `CaseFold` also has Bob
/// commit once as `Bob@Y.io` and maps `<BOB@y.IO>` to him: mailmap matches e-mails without
/// regard to case, and a store that keys identities must agree.
pub fn matrix_repo(dir: &Path, mailmap: Mailmap) -> (std::path::PathBuf, Values) {
    let adv = crate::Adversarial::build(dir);
    let f = crate::Fixture::init(&adv.repo);
    let step =
        |f: crate::Fixture, at: &str, who: (&str, &str), files: &[(&str, &str)], msg: &str| {
            f.git(&["config", "user.name", who.0]);
            f.git(&["config", "user.email", who.1]);
            let f = files.iter().fold(f.at(at), |f, (p, b)| f.write(p, b));
            f.git(&["add", "-f", "-A"]);
            f.git(&["commit", "-q", "--allow-empty", "-m", msg]);
            f
        };
    let jane = ("Jane Doe", "jane@x.io");
    let bob = ("Bob", "bob@y.io");
    let bot = (
        "dependabot[bot]",
        "49699333+dependabot[bot]@users.noreply.github.com",
    );
    let f = step(
        f,
        "2023-06-01T10:00:00Z",
        bob,
        &[("src/lib.rs", "fn a() {}\nfn b() {}\n")],
        "feat(core): year",
    );
    let f = step(
        f,
        "2024-01-15T10:00:00Z",
        bot,
        &[("Cargo.lock", "a\nb\nc\n")],
        "chore(deps): bump",
    );
    let f = step(
        f,
        "2024-02-20T10:00:00Z",
        bob,
        &[(".gitignore", "build/\n"), ("build/out.txt", "o\n")],
        "chore: ignored output",
    );
    let f = step(
        f,
        "2024-02-25T10:00:00Z",
        jane,
        &[("vendor/x.c", "int x;\nint y;\n")],
        "feat(vendor): vendored",
    );
    let f = step(
        f,
        "2024-03-04T10:00:00Z",
        bob,
        &[("src/lib.rs", "fn a() {}\n\nfn b() {}\n")],
        "style(core): blank line",
    );
    let f = if mailmap == Mailmap::CaseFold {
        step(
            f,
            "2024-03-06T10:00:00Z",
            ("Bob", "Bob@Y.io"),
            &[("docs/y.md", "y\n")],
            "docs: case",
        )
    } else {
        f
    };
    let f = step(
        f,
        "2024-03-08T10:00:00Z",
        jane,
        &[
            ("src/lib.rs", "fn a() {}\n\nfn b() { 1 }\n"),
            ("docs/x.md", "d\n"),
        ],
        "fix(core): week",
    );
    let edited = "Jane Doe <jane@x.io> <old@x.io>\nBob B <bob@y.io> Bob <bob@y.io>\n";
    let folded = format!("{edited}Bob B <bob@y.io> <BOB@y.IO>\n");
    let f = match mailmap {
        Mailmap::AsIs => f,
        Mailmap::Edited => step(
            f,
            "2024-03-09T10:00:00Z",
            jane,
            &[(".mailmap", edited)],
            "docs: edit mailmap",
        ),
        Mailmap::CaseFold => step(
            f,
            "2024-03-09T10:00:00Z",
            jane,
            &[(".mailmap", &folded)],
            "docs: edit mailmap",
        ),
    };
    let repo = f.path().to_owned();
    let values = Values {
        author: "bob@y.io".to_owned(),
        kind: "fix".to_owned(),
        scope: "core".to_owned(),
        path: "src".to_owned(),
        exclude: "vendor/**".to_owned(),
    };
    (repo, values)
}

/// Waits until the insights pages show data instead of the indexing progress.
pub async fn ready(router: &Router) {
    ready_at(router, "HEAD").await;
}

/// Waits until the insights of `rev` show data instead of the indexing progress: a revision other
/// than HEAD loads its own record set on the first visit.
pub async fn ready_at(router: &Router, rev: &str) {
    let probe = format!("/stats?rev={}", crate::matrix::enc(rev));
    for _ in 0..600 {
        let (_, body) = page(router, &probe).await;
        if !body.contains("data-progress") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("index never became ready");
}

/// The page and API paths every cell asks for. `pages` are the insights pages (the web
/// crate's `WARM_PAGES`), `insights` every API insight name.
#[must_use]
pub fn targets(pages: &[&str], insights: &[&str]) -> Vec<String> {
    let mut t: Vec<String> = pages.iter().map(|p| (*p).to_owned()).collect();
    t.extend(insights.iter().map(|n| format!("/api/v1/insights/{n}")));
    t.push("/api/v1/contributors".to_owned());
    t
}

/// Every answer of `targets` under every cell, keyed `target?query`. A non-200 answer is kept
/// as `status N` plus the body, so an error is an answer too.
pub async fn capture(
    router: &Router,
    targets: &[String],
    cells: &[Cell],
    values: &Values,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for cell in cells {
        let q = cell.query(values);
        for t in targets {
            let key = format!("{t}?{q}");
            let (status, body) = page(router, &key).await;
            let body = normalize(&body);
            let v = if status == 200 {
                body
            } else {
                format!("status {status}\n{body}")
            };
            out.insert(key, v);
        }
    }
    out
}

/// [`capture`] with `rev=<rev>` appended to every query: the page or API for that revision.
pub async fn capture_rev(
    router: &Router,
    targets: &[String],
    cells: &[Cell],
    values: &Values,
    rev: &str,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for cell in cells {
        let q = format!("{}&rev={}", cell.query(values), crate::matrix::enc(rev));
        for t in targets {
            let key = format!("{t}?{q}");
            let (status, body) = page(router, &key).await;
            let body = normalize(&body);
            let v = if status == 200 {
                body
            } else {
                format!("status {status}\n{body}")
            };
            out.insert(key, v);
        }
    }
    out
}

/// The page with its chrome cut out.
///
/// That is the site `<header>`, and for the overview (`/`) the `<aside>` sidebar too. Header
/// and sidebar change with navigation work, the insights never do, so a re-bless may move only
/// what this removes.
#[must_use]
pub fn without_chrome(key: &str, body: &str) -> String {
    let path = key.split_once("] ").map_or(key, |(_, k)| k);
    let overview = path.split('?').next() == Some("/");
    let mut out = cut(body, "<header", "</header>");
    if overview {
        out = cut(&out, "<aside", "</aside>");
    }
    out
}

/// `body` without its first `open ... close` span, unchanged when either is missing.
fn cut(body: &str, open: &str, close: &str) -> String {
    let Some(at) = body.find(open) else {
        return body.to_owned();
    };
    let Some(end) = body[at..].find(close).map(|e| at + e + close.len()) else {
        return body.to_owned();
    };
    format!("{}{}", &body[..at], &body[end..])
}

/// Keys whose body outside the chrome differs between two dumps.
///
/// The dumps are `TEST_ORACLE_DUMP` folders. Keys on one side only count too. Empty when a
/// change touched only header and sidebar. Equal-time row order is folded, as everywhere in the
/// oracle.
#[must_use]
pub fn chrome_only_diff(old_dir: &Path, new_dir: &Path) -> (usize, Vec<String>) {
    let read = |dir: &Path| -> BTreeMap<String, String> {
        std::fs::read_dir(dir)
            .expect("dump dir")
            .filter_map(Result::ok)
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| {
                t.split_once('\n')
                    .map(|(k, b)| (k.to_owned(), normalize(b)))
            })
            .collect()
    };
    let (old, new) = (read(old_dir), read(new_dir));
    let mut moved = 0;
    let mut bad = Vec::new();
    let mut keys: std::collections::BTreeSet<&String> = old.keys().collect();
    keys.extend(new.keys());
    for k in keys {
        match (old.get(k), new.get(k)) {
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b))
                if canonical(&without_chrome(k, a)) == canonical(&without_chrome(k, b)) =>
            {
                moved += 1;
            }
            _ => bad.push(k.clone()),
        }
    }
    (moved, bad)
}

/// Blanks the one value that is not output of the insights but of the watcher: `data-live-v`,
/// a hash of worktree file stamps that differs per checkout. Nothing else is touched.
#[must_use]
pub fn normalize(body: &str) -> String {
    const KEY: &str = "data-live-v=\"";
    let Some(at) = body.find(KEY) else {
        return body.to_owned();
    };
    let start = at + KEY.len();
    let end = body[start..].find('"').map_or(body.len(), |e| start + e);
    format!("{}_{}", &body[..start], &body[end..])
}

/// The sort key of a list row: the `data-tip` of its `<time>`, which holds the date to the
/// minute, plus the shown relative text. `None` for a line that is not a dated row.
fn row_key(line: &str) -> Option<&str> {
    let at = line.find("<time data-tip=\"")?;
    let rest = &line[at..];
    rest.find("</time>").map(|e| &rest[..e])
}

/// A body cut into units: each line, further cut before every `<li`, so rows a template
/// renders on one line are still compared row by row.
fn units(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for line in body.lines() {
        let mut rest = line;
        while let Some(at) = rest.get(1..).and_then(|r| r.find("<li")).map(|i| i + 1) {
            out.push(&rest[..at]);
            rest = &rest[at..];
        }
        out.push(rest);
    }
    out
}

/// Whether `new` differs from `old` only by reordering rows inside runs of equal sort key.
///
/// That is: the same unit count (lines, cut before each `<li`), every changed unit a dated row, the
/// sequence of keys unchanged, and in each run of consecutive rows with one key the same multiset
/// of rows. Anything else, including a row that moved across a different time, is `false`.
#[must_use]
pub fn tie_only(old: &str, new: &str) -> bool {
    let before = units(old);
    let after = units(new);
    if before.len() != after.len() || before == after {
        return false;
    }
    let mut at = 0;
    while at < before.len() {
        if before[at] == after[at] {
            at += 1;
            continue;
        }
        let Some(key) = row_key(before[at]) else {
            return false;
        };
        // The run: consecutive rows with this key, in both bodies at the same positions.
        let mut start = at;
        while start > 0 && row_key(before[start - 1]) == Some(key) {
            start -= 1;
        }
        let mut end = at;
        while end < before.len() && row_key(before[end]) == Some(key) {
            end += 1;
        }
        if (start..end).any(|row| row_key(after[row]) != Some(key)) {
            return false;
        }
        let mut was = before[start..end].to_vec();
        let mut now = after[start..end].to_vec();
        was.sort_unstable();
        now.sort_unstable();
        if was != now {
            return false;
        }
        at = end;
    }
    true
}

/// One golden line: `exact canonical key`, the canonical digest being [`canonical`]'s.
pub(crate) fn golden_line(l: &str) -> Option<(&str, (&str, &str))> {
    let (exact, rest) = l.split_once(' ')?;
    let (canon, key) = rest.split_once(' ')?;
    Some((key, (exact, canon)))
}

/// Prints the tie-only count per page (the key without its option query).
fn report_ties(ties: &[String]) {
    let mut per: BTreeMap<&str, usize> = BTreeMap::new();
    for k in ties {
        *per.entry(k.split('?').next().unwrap_or(k)).or_default() += 1;
    }
    let mut out = std::io::stderr().lock();
    // The tie-only summary goes to stderr, next to the per-cell report.
    writeln!(out, "tie-only reorders: {} cells", ties.len()).expect("stderr");
    for (page, n) in per {
        writeln!(out, "  {n:5} {page}").expect("stderr");
    }
}

/// The body with every run of consecutive rows of one sort key sorted: two bodies that differ
/// only by order within equal-time runs have the same canonical form, and nothing else does.
#[must_use]
pub fn canonical(body: &str) -> String {
    let all = units(body);
    let mut out: Vec<&str> = Vec::with_capacity(all.len());
    let mut at = 0;
    while at < all.len() {
        let Some(key) = row_key(all[at]) else {
            out.push(all[at]);
            at += 1;
            continue;
        };
        let mut end = at;
        while end < all.len() && row_key(all[end]) == Some(key) {
            end += 1;
        }
        let mut run = all[at..end].to_vec();
        run.sort_unstable();
        out.extend(run);
        at = end;
    }
    out.join("\n")
}

/// FNV-1a 128: stable across platforms and releases, unlike `DefaultHasher`.
#[must_use]
pub fn digest(body: &str) -> String {
    let mut h: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for b in body.bytes() {
        h ^= u128::from(b);
        h = h.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    format!("{h:032x}")
}

/// Compares `answers` to the golden file at `golden` (`exact canonical key` per line).
///
/// An answer passes when its exact digest matches, or when only equal-time rows moved (the
/// canonical digest matches); those are counted per page on stderr. Under `TEST_BLESS=1` writes the
/// file instead. On a mismatch writes every differing body to
/// `dump` (a folder under `target/`, the caller's `CARGO_TARGET_TMPDIR`) and panics with the
/// first keys and that path. A green run removes a stale `dump`.
pub fn check(golden: &Path, dump: &Path, answers: &BTreeMap<String, String>) {
    let mut text = String::new();
    for (k, v) in answers {
        writeln!(text, "{} {} {k}", digest(v), digest(&canonical(v))).expect("write string");
    }
    if let Some(dir) = std::env::var_os("TEST_ORACLE_DUMP") {
        // Every body, one file per answer, to diff two runs when a digest moves.
        let dir = Path::new(&dir);
        std::fs::create_dir_all(dir).expect("mkdir dump");
        for (k, v) in answers {
            let name = digest(k);
            std::fs::write(dir.join(name), format!("{k}\n{v}")).expect("dump");
        }
    }
    if std::env::var_os("TEST_BLESS").is_some() {
        // Only the reference build blesses: today's code, never the code under test. Each line
        // keeps the exact digest and the digest of the body with equal-time runs sorted, so a
        // later implementation may order ties differently and nothing else.
        let header: String = std::env::var("TEST_ORACLE_REASON")
            .map(|r| format!("# {r}\n"))
            .unwrap_or_default();
        std::fs::create_dir_all(golden.parent().expect("parent")).expect("mkdir");
        std::fs::write(golden, format!("{header}{text}")).expect("write golden");
        return;
    }
    let want = std::fs::read_to_string(golden).unwrap_or_else(|e| {
        panic!(
            "{}: {e}. Run with TEST_BLESS=1 on today's code",
            golden.display()
        )
    });
    let want: BTreeMap<&str, (&str, &str)> = want
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(golden_line)
        .collect();
    let mut bad: Vec<String> = Vec::new();
    let mut ties: Vec<String> = Vec::new();
    let mut keys: std::collections::BTreeSet<&str> = want.keys().copied().collect();
    keys.extend(answers.keys().map(String::as_str));
    for k in keys {
        match (want.get(k), answers.get(k)) {
            (Some((exact, _)), Some(v)) if *exact == digest(v) => {}
            (Some((_, canon)), Some(v)) if *canon == digest(&canonical(v)) => {
                ties.push(k.to_owned());
            }
            _ => bad.push(k.to_owned()),
        }
    }
    if !ties.is_empty() {
        report_ties(&ties);
    }
    if bad.is_empty() {
        if dump.exists() {
            std::fs::remove_dir_all(dump).expect("remove stale dump");
        }
        return;
    }
    if dump.exists() {
        std::fs::remove_dir_all(dump).expect("remove old dump");
    }
    std::fs::create_dir_all(dump).expect("mkdir dump");
    for (i, k) in bad.iter().enumerate().take(200) {
        let body = answers.get(k).map_or("(missing)", String::as_str);
        std::fs::write(dump.join(format!("{i:03}.txt")), format!("{k}\n{body}")).expect("dump");
    }
    panic!(
        "{} of {} answers differ from {} (bodies in {}), first: {:?}",
        bad.len(),
        want.len().max(answers.len()),
        golden.display(),
        dump.display(),
        &bad[..bad.len().min(5)]
    );
}

#[cfg(test)]
mod tests;
