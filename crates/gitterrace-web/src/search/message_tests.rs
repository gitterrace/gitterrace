//! Commit search through the message column against the walk it replaces: the same hits in
//! the same order, the setting off unchanged, a damaged column rebuilt and never served, a new
//! commit found after the build, and a miss that reads no commit from git.

use std::path::Path;
use std::sync::Arc;

use regex::Regex;

use crate::app::{App, Env};
use crate::search::build_regex;
use crate::search::commits::{search_commits, search_indexed, walk_commits};

/// Queries that hit and miss: subjects, bodies, authors before and after the mailmap, the
/// lossy replacement of a non-UTF-8 byte, anchors and the message/author seam.
const QUERIES: &[(&str, bool)] = &[
    ("fix", false),
    ("feat", false),
    ("Fix", true),
    ("^Merge", false),
    ("body line", false),
    ("Jane Doe", false),
    ("old@x.io", false),
    ("jane@x.io", false),
    ("jdoe", false),
    ("na\u{fffd}ve", false),
    ("\u{fffd}", false),
    ("\n.*<", false),
    (r"tip\n", false),
    ("^", false),
    ("no such text anywhere", false),
    ("zzzz", false),
    (r"\d{3}", false),
];

fn app(repo: &Path, on: bool, cap: usize) -> Arc<App> {
    let repo = gitterrace_git::Repo::discover(repo).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let mut cfg = gitterrace_config::Config::default();
    cfg.index.has_messages = on;
    cfg.limits.search_commits = cap;
    Arc::new(App::configured(repo, index, &Env::new(cfg)))
}

fn build(a: &App) {
    a.index.update(&a.repo, "HEAD", &|_| {}).expect("update");
}

fn ids(rows: &[gitterrace_git::CommitSummary]) -> Vec<String> {
    rows.iter().map(|c| c.id.clone()).collect()
}

fn re(q: &str, case: bool) -> Regex {
    build_regex(q, true, case).expect("regex")
}

/// Every query on `a`: the indexed answer equals the walk, row for row.
fn assert_equal(a: &App, what: &str) {
    for (q, case) in QUERIES {
        let r = re(q, *case);
        let (walk, walk_capped) = walk_commits(a, "HEAD", &r).expect("walk");
        let hits = search_indexed(a, "HEAD", &r, a.cfg().limits.search_commits)
            .expect("indexed")
            .unwrap_or_else(|| panic!("{what}: the column answers {q:?}"));
        assert_eq!(hits.ids, ids(&walk), "{what}: hits of {q:?}");
        assert_eq!(hits.capped, walk_capped, "{what}: cap of {q:?}");
        let (rows, _) = search_commits(a, "HEAD", &r).expect("search");
        assert_eq!(
            format!("{rows:?}"),
            format!("{walk:?}"),
            "{what}: rows of {q:?}"
        );
    }
}

/// Merges whose dates disagree with the graph, a mailmap, a non-UTF-8 message and a body.
fn mixed(dir: &Path) -> std::path::PathBuf {
    let repo = dir.join("mixed");
    let f = gitterrace_testkit::Fixture::init(&repo).at("2024-01-01T12:00:00Z");
    f.git(&["config", "user.name", "jdoe"]);
    f.git(&["config", "user.email", "old@x.io"]);
    let f = f.write("a", "1").commit("feat: start\n\nbody line one");
    let f = f.branch("side").at("2024-03-01T12:00:00Z");
    let f = f.write("s", "1").commit("Fix: side newer than main");
    let f = f.checkout("main").at("2024-02-01T12:00:00Z");
    let f = f.write("m", "1").commit("fix: main older");
    f.git(&["merge", "-q", "--no-ff", "side", "-m", "Merge side"]);
    // Same second as the merge: ties in the walk's queue.
    let f = f.write("t", "1").commit("fix: tie 101");
    let f = f.write("t", "2").commit("fix: tie 102");
    f.git(&["config", "i18n.commitEncoding", "latin1"]);
    let raw = repo.join("latin1.txt");
    std::fs::write(&raw, b"docs: na\xefve tip\n").expect("write");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-q", "-F", raw.to_str().expect("utf8")]);
    f.git(&["config", "user.name", "Jane Doe"]);
    f.git(&["config", "user.email", "jane@x.io"]);
    let f = f
        .write(".mailmap", "Jane Doe <jane@x.io> <old@x.io>\n")
        .commit("docs: mailmap");
    drop(f);
    repo
}

#[test]
fn the_column_lists_what_the_walk_lists() {
    let dir = tempfile::tempdir().expect("tmp");
    let mixed = mixed(dir.path());
    let adv = gitterrace_testkit::Adversarial::build(&dir.path().join("adv"));
    for (what, repo) in [
        ("mixed", mixed.as_path()),
        ("adversarial", adv.repo.as_path()),
        ("shallow", adv.shallow.as_path()),
    ] {
        for cap in [200, 2] {
            let a = app(repo, true, cap);
            build(&a);
            assert_equal(&a, &format!("{what}, cap {cap}"));
        }
    }
}

#[test]
fn an_empty_repository_answers_by_the_walk() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = gitterrace_testkit::Fixture::init(dir.path());
    let a = app(f.path(), true, 200);
    // Nothing to index: the update fails or does nothing, and both searches fail alike.
    drop(a.index.update(&a.repo, "HEAD", &|_| {}));
    assert!(search_commits(&a, "HEAD", &re("x", false)).is_err());
    assert!(search_indexed(&a, "HEAD", &re("x", false), 200).is_err());
}

#[test]
fn turned_off_the_search_walks_git_and_the_column_goes() {
    let dir = tempfile::tempdir().expect("tmp");
    let repo = mixed(dir.path());
    let on = app(&repo, true, 200);
    build(&on);
    let names = |a: &App| -> Vec<String> {
        let m = gitterrace_index::seg::manifest::Manifest::load(a.index.dir())
            .expect("manifest")
            .expect("set");
        m.live
            .into_iter()
            .map(|f| f.name)
            .filter(|n| n.starts_with("msg-"))
            .collect()
    };
    assert_eq!(names(&on).len(), 1, "on builds one column file");
    drop(on);
    let off = app(&repo, false, 200);
    let r = re("fix", false);
    assert!(
        search_indexed(&off, "HEAD", &r, 200)
            .expect("off")
            .is_none()
    );
    let before = off.repo.commits_read();
    let (rows, _) = search_commits(&off, "HEAD", &r).expect("search");
    assert!(
        off.repo.commits_read() > before,
        "off reads git, as before the column"
    );
    assert_eq!(
        ids(&rows),
        ids(&walk_commits(&off, "HEAD", &r).expect("walk").0)
    );
    build(&off);
    assert!(
        names(&off).is_empty(),
        "off drops the column on the next update"
    );
    assert!(
        !std::fs::read_dir(off.index.dir()).expect("dir").any(|e| e
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .starts_with("msg-")),
        "and deletes its file"
    );
}

#[test]
fn a_damaged_column_is_rebuilt_never_served() {
    let dir = tempfile::tempdir().expect("tmp");
    let repo = mixed(dir.path());
    let r = re("fix", false);
    let a = app(&repo, true, 200);
    build(&a);
    let want = ids(&walk_commits(&a, "HEAD", &r).expect("walk").0);
    let file = |a: &App| {
        let m = gitterrace_index::seg::manifest::Manifest::load(a.index.dir())
            .expect("manifest")
            .expect("set");
        let f = m
            .live
            .into_iter()
            .find(|f| f.name.starts_with("msg-"))
            .expect("column");
        a.index.dir().join(f.name)
    };
    let len = std::fs::metadata(file(&a)).expect("meta").len();
    let damage: [&dyn Fn(&Path); 4] = [
        // Cut short: the manifest's length check refuses it on open.
        &|p| {
            let f = std::fs::OpenOptions::new()
                .write(true)
                .open(p)
                .expect("open");
            f.set_len(len / 2).expect("cut");
        },
        // One flipped byte in the first block: only its crc can tell.
        &|p| {
            let mut b = std::fs::read(p).expect("read");
            b[20] ^= 0x55;
            std::fs::write(p, b).expect("write");
        },
        // The tail of the last block zeroed, length and footer kept, as a torn write leaves
        // it: only the block crc can tell.
        &|p| {
            let mut b = std::fs::read(p).expect("read");
            let toc = b.len() - 64 - 24;
            b[toc - 16..toc].fill(0);
            std::fs::write(p, b).expect("write");
        },
        &|p| std::fs::remove_file(p).expect("remove"),
    ];
    for (i, d) in damage.iter().enumerate() {
        a.mark_ready();
        d(&file(&a));
        let (rows, _) = search_commits(&a, "HEAD", &r).expect("search");
        assert_eq!(
            ids(&rows),
            want,
            "damage {i}: the walk answers this request whole"
        );
        assert!(!a.ready(), "damage {i}: the rebuild started");
        assert!(
            gitterrace_index::seg::manifest::Manifest::load(a.index.dir())
                .expect("manifest")
                .is_none(),
            "damage {i}: the cache was wiped"
        );
        build(&a);
        assert_equal(&a, &format!("rebuilt after damage {i}"));
    }
}

#[test]
fn a_commit_after_the_build_is_found() {
    let dir = tempfile::tempdir().expect("tmp");
    let repo = mixed(dir.path());
    let a = app(&repo, true, 200);
    build(&a);
    let r = re("brand new needle", false);
    assert_eq!(
        search_indexed(&a, "HEAD", &r, 200)
            .expect("indexed")
            .map(|h| h.ids.len()),
        Some(0)
    );
    let f = gitterrace_testkit::Fixture::init(&repo);
    drop(f.write("n", "1").commit("feat: brand new needle"));
    // Before the update the column is not current: the walk answers, never a stale list.
    assert!(
        search_indexed(&a, "HEAD", &r, 200)
            .expect("stale")
            .is_none()
    );
    assert_eq!(search_commits(&a, "HEAD", &r).expect("walk").0.len(), 1);
    build(&a);
    let hits = search_indexed(&a, "HEAD", &r, 200)
        .expect("indexed")
        .expect("current");
    assert_eq!(hits.ids, vec![a.repo.resolve("HEAD").expect("head")]);
    assert_equal(&a, "after an append");
}

#[test]
fn a_miss_through_the_column_reads_no_commit() {
    let dir = tempfile::tempdir().expect("tmp");
    let repo = mixed(dir.path());
    let a = app(&repo, true, 200);
    build(&a);
    let before = a.repo.commits_read();
    let (rows, capped) =
        search_commits(&a, "HEAD", &re("no such text anywhere", false)).expect("search");
    assert!(rows.is_empty() && !capped);
    assert_eq!(a.repo.commits_read(), before, "a miss reads no commit");
    let (rows, _) = search_commits(&a, "HEAD", &re("tie", false)).expect("search");
    assert_eq!(
        a.repo.commits_read() - before,
        rows.len() as u64,
        "a hit reads only the commits it lists"
    );
}
