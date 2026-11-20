//! Compare reaches branches the served head does not. The index holds what the head reaches,
//! so a side branch's own commits are read from git: comparing `main` with it lists them and
//! counts their lines. An empty range says so and offers the other direction.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::fmt::Write as _;

use gitterrace_testkit::{Fixture, page};

#[tokio::test(flavor = "multi_thread")]
async fn a_side_branch_compares_and_an_empty_range_says_so() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path())
        .at("2024-03-01T10:00:00Z")
        .write("a.rs", "fn a() {}\n")
        .commit("feat(core): start")
        .branch("side")
        .at("2024-03-02T10:00:00Z")
        .write("b.py", "x = 1\ny = 2\n")
        .commit("feat(core): side work")
        .checkout("main")
        .at("2024-03-03T10:00:00Z")
        .write("a.rs", "fn a() {}\nfn b() {}\n")
        .commit("fix(core): main work");
    let router = crate::common::ready_router(f.path()).await;

    let (status, html) = page(&router, "/compare?base=main&head=side").await;
    assert_eq!(status, 200);
    assert!(html.contains("side work"), "the side commit is listed");
    assert!(html.contains("<b>1</b><span>Commits without merges"), "one commit counted");
    assert!(html.contains("<th scope=\"row\">Python</th>"), "its language is counted");

    let (_, same) = page(&router, "/compare?base=main&head=main").await;
    assert!(same.contains("class=\"empty\""), "an empty range shows the empty state");
    assert!(same.contains("has no commits that"), "and says why");

    // Without a tag to start from, the page still says what to do and offers branches.
    let (_, start) = page(&router, "/compare?head=main").await;
    assert!(start.contains("Choose a base"), "an untagged start explains itself");
    assert!(start.contains("compare?base=side&#38;head=main"), "and links the other branch");
}

/// `side` with `n` commits of one line each, none reachable from `main`, made in one
/// fast-import so a range past the cap costs milliseconds.
fn long_side(n: usize) -> (tempfile::TempDir, Fixture) {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path())
        .at("2024-03-01T10:00:00Z")
        .write("a.rs", "fn a() {}\n")
        .commit("feat(core): start");
    let mut stream = String::new();
    for i in 0..n {
        let body = format!("{i}\n");
        let from = if i == 0 { "from refs/heads/main\n" } else { "" };
        let _ = write!(
            stream,
            "commit refs/heads/side\ncommitter T <t@x> {} +0000\ndata 4\nside\n{from}\
             M 100644 inline n.txt\ndata {}\n{body}\n",
            1_709_300_000 + i,
            body.len()
        );
    }
    gitterrace_testkit::git_stdin(f.path(), &["fast-import", "--quiet"], stream.as_bytes());
    (dir, f)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_range_past_the_cap_asks_for_a_nearer_base() {
    let (_dir, f) = long_side(2_001);
    let router = crate::common::ready_router(f.path()).await;
    let t = std::time::Instant::now();
    let (status, html) = page(&router, "/compare?base=main&head=side").await;
    assert_eq!(status, 200);
    assert!(html.contains("too many to read here"), "the bounded state shows");
    assert!(!html.contains("Commits without merges"), "and no partial figures");
    assert!(t.elapsed().as_secs() < 10, "headers only: {:?}", t.elapsed());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_git_read_shows_an_error_not_zeros() {
    let (_dir, f) = long_side(3);
    // Loose objects, so one can go missing.
    let packs: Vec<_> = std::fs::read_dir(f.path().join(".git/objects/pack"))
        .expect("pack dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "pack"))
        .collect();
    for pack in packs {
        let moved = pack.with_extension("moved");
        std::fs::rename(&pack, &moved).expect("move pack");
        std::fs::remove_file(pack.with_extension("idx")).expect("rm idx");
        let data = std::fs::read(&moved).expect("read pack");
        gitterrace_testkit::git_stdin(f.path(), &["unpack-objects", "-q"], &data);
        std::fs::remove_file(&moved).expect("rm moved");
    }
    // The side tip's tree goes missing: its commit still walks, its diff cannot be read.
    let tree = f.git(&["rev-parse", concat!("side^", "{", "tree}")]);
    let obj = f.path().join(".git/objects").join(&tree[..2]).join(&tree[2..]);
    std::fs::remove_file(&obj).expect("loose tree after fast-import unpack");
    let router = crate::common::ready_router(f.path()).await;
    let (status, html) = page(&router, "/compare?base=main&head=side").await;
    assert_eq!(status, 200);
    assert!(html.contains("could not be read"), "the failure is said");
    assert!(!html.contains("<b>0</b><span>Commits"), "never zeros");
}
