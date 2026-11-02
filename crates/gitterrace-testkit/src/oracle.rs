//! The equivalence oracle: today's answers of every insights page and API, captured over the
//! generated option matrix with the clock frozen, kept as one golden digest per answer.
//!
//! The golden file is the reference. A later implementation passes when every digest is the
//! same. `TEST_BLESS=1` rewrites the file, which only today's implementation may do.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use axum::Router;

use crate::matrix::{Cell, Mailmap, Values};
use crate::web::page;

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
/// A bot, a lock file (generated), a tracked file `.gitignore` rules out, a whitespace-only change, a vendored
/// directory (the exclude glob) and dates inside each period window before [`FROZEN_NOW`].
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

/// Waits until the insights pages no longer show the indexing progress.
pub async fn ready(router: &Router) {
    for _ in 0..600 {
        let (_, body) = page(router, "/stats").await;
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
/// That is: the same unit count (lines, cut before each `<li`), every changed unit a dated row, the sequence of keys unchanged, and
/// in each run of consecutive rows with one key the same multiset of rows. Anything else,
/// including a row that moved across a different time, is `false`.
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

/// Keys whose digest differs between the golden text `prev` and `answers`, or that exist on
/// one side only.
fn moved_keys(prev: &str, answers: &BTreeMap<String, String>) -> Vec<String> {
    let want: BTreeMap<&str, &str> = prev
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once(' ').map(|(d, k)| (k, d)))
        .collect();
    let mut keys: std::collections::BTreeSet<&str> = want.keys().copied().collect();
    keys.extend(answers.keys().map(String::as_str));
    keys.into_iter()
        .filter(|k| want.get(k).copied() != answers.get(*k).map(|v| digest(v)).as_deref())
        .map(str::to_owned)
        .collect()
}

/// Prints the tie-only count per page (the key without its option query).
fn report_ties(ties: &[String]) {
    let mut per: BTreeMap<&str, usize> = BTreeMap::new();
    for k in ties {
        *per.entry(k.split('?').next().unwrap_or(k)).or_default() += 1;
    }
    let mut out = std::io::stderr().lock();
    // The re-bless report a re-bless records with the golden: stderr is its channel.
    writeln!(out, "tie-only reorders: {} cells", ties.len()).expect("stderr");
    for (page, n) in per {
        writeln!(out, "  {n:5} {page}").expect("stderr");
    }
}

/// Splits the keys whose digest moved into tie-only reorders and real differences.
///
/// Reads the old bodies from `old_dir` (a `TEST_ORACLE_DUMP` of the reference run). Returns
/// (tie-only keys, other keys). A key without an old body counts as a real difference.
#[must_use]
pub fn classify(
    old_dir: &Path,
    answers: &BTreeMap<String, String>,
    moved: &[String],
) -> (Vec<String>, Vec<String>) {
    let (mut ties, mut real) = (Vec::new(), Vec::new());
    for k in moved {
        let old = std::fs::read_to_string(old_dir.join(digest(k)))
            .ok()
            .and_then(|b| b.split_once('\n').map(|(_, body)| normalize(body)));
        match (old, answers.get(k)) {
            (Some(o), Some(n)) if tie_only(&o, n) => ties.push(k.clone()),
            _ => real.push(k.clone()),
        }
    }
    (ties, real)
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

/// Compares `answers` to the golden file at `golden` (one `digest key` line each).
///
/// Under `TEST_BLESS=1` writes the file instead. On a mismatch writes every differing body to
/// `dump` (a folder under `target/`, the caller's `CARGO_TARGET_TMPDIR`) and panics with the
/// first keys and that path. A green run removes a stale `dump`.
pub fn check(golden: &Path, dump: &Path, answers: &BTreeMap<String, String>) {
    let mut text = String::new();
    for (k, v) in answers {
        writeln!(text, "{} {k}", digest(v)).expect("write string");
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
        // A re-bless over an existing golden must name the reference bodies, so only tie-only
        // reorders can move a digest. A first bless (no golden yet) needs none.
        if let Ok(prev) = std::fs::read_to_string(golden) {
            let old = std::env::var_os("TEST_ORACLE_OLD").unwrap_or_else(|| {
                panic!("re-bless needs TEST_ORACLE_OLD=<TEST_ORACLE_DUMP of the reference run>")
            });
            let moved = moved_keys(&prev, answers);
            let (ties, real) = classify(Path::new(&old), answers, &moved);
            report_ties(&ties);
            assert!(
                real.is_empty(),
                "re-bless refused, {} real differences: {:?}",
                real.len(),
                &real[..real.len().min(5)]
            );
        }
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
    let want: BTreeMap<&str, &str> = want
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once(' ').map(|(d, k)| (k, d)))
        .collect();
    let got: BTreeMap<&str, String> = answers
        .iter()
        .map(|(k, v)| (k.as_str(), digest(v)))
        .collect();
    let mut bad: Vec<String> = Vec::new();
    let mut keys: std::collections::BTreeSet<&&str> = want.keys().collect();
    keys.extend(got.keys());
    for k in keys {
        if want.get(k).copied() != got.get(k).map(String::as_str) {
            bad.push((*k).to_owned());
        }
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
        want.len().max(got.len()),
        golden.display(),
        dump.display(),
        &bad[..bad.len().min(5)]
    );
}

#[cfg(test)]
mod tests {
    use crate::oracle::{digest, tie_only};

    fn row(time: &str, who: &str) -> String {
        format!(
            "<li><b>{who}</b> <time data-tip=\"{time}\" tabindex=\"0\">2 months ago</time></li>"
        )
    }

    #[test]
    fn a_swap_within_equal_times_is_tie_only() {
        let a = [
            "<ul>".to_owned(),
            row("2024-01-01 12:00", "a"),
            row("2024-01-01 12:00", "b"),
            row("2023-01-01 12:00", "c"),
            "</ul>".to_owned(),
        ];
        let b = [
            "<ul>".to_owned(),
            row("2024-01-01 12:00", "b"),
            row("2024-01-01 12:00", "a"),
            row("2023-01-01 12:00", "c"),
            "</ul>".to_owned(),
        ];
        assert!(tie_only(&a.join("\n"), &b.join("\n")));
    }

    /// The red proof: two rows of different times swapped, a changed row, a changed non-row
    /// line, and an added row all fail.
    #[test]
    fn anything_but_a_tie_reorder_is_a_real_difference() {
        let base = [
            "<ul>".to_owned(),
            row("2024-01-01 12:00", "a"),
            row("2023-01-01 12:00", "c"),
            "</ul>".to_owned(),
        ];
        let old = base.join("\n");
        let swapped = [
            "<ul>".to_owned(),
            base[2].clone(),
            base[1].clone(),
            "</ul>".to_owned(),
        ];
        assert!(!tie_only(&old, &swapped.join("\n")), "cross-time swap");
        let changed = [
            "<ul>".to_owned(),
            row("2024-01-01 12:00", "z"),
            base[2].clone(),
            "</ul>".to_owned(),
        ];
        assert!(!tie_only(&old, &changed.join("\n")), "changed row");
        assert!(
            !tie_only(&old, &old.replace("<ul>", "<ol>")),
            "changed markup"
        );
        assert!(
            !tie_only(&old, &format!("{old}\n{}", row("2022-01-01 12:00", "d"))),
            "added row"
        );
        assert!(!tie_only(&old, &old), "no change is not a tie reorder");
    }

    /// Rows on one line, as the commit list renders them: the same rules hold per row.
    #[test]
    fn rows_on_one_line_compare_row_by_row() {
        let one = |rows: &[String]| format!("<ul>{}</ul>", rows.concat());
        let (a, b, c) = (
            row("2024-01-01 12:00", "a"),
            row("2024-01-01 12:00", "b"),
            row("2023-01-01 12:00", "c"),
        );
        let old = one(&[a.clone(), b.clone(), c.clone()]);
        assert!(tie_only(&old, &one(&[b.clone(), a.clone(), c.clone()])));
        assert!(
            !tie_only(&old, &one(&[a, c, b])),
            "cross-time swap on one line"
        );
    }

    #[test]
    fn digest_is_pinned() {
        assert_eq!(digest(""), "6c62272e07bb014262b821756295c58d");
        assert_ne!(digest("a"), digest("b"));
    }
}
