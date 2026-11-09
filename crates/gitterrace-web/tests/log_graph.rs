//! The Graph view of the log (`?view=graph`): a decorative cell of lines beside each commit
//! row, lines that run on from one page to the next, the List view without a graph, and the
//! "+N" folds. `TEST_GRAPH_REPO=/path/to/linux.git` adds the wide-history check: on its first
//! pages every row folds exactly the columns past 3 (narrow) and past 8 (wide) that it draws.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use gitterrace_testkit::{git, git_at, page};

/// Each commit a second later than the last, so the log's time order is the order made.
static CLOCK: AtomicU32 = AtomicU32::new(0);

fn at() -> String {
    format!(
        "{} +0000",
        1_700_000_000 + CLOCK.fetch_add(1, Ordering::Relaxed)
    )
}

fn commit(dir: &Path, msg: &str) {
    git_at(dir, &at(), &["commit", "-q", "--allow-empty", "-m", msg]);
}

fn merge(dir: &Path, branch: &str) {
    let msg = format!("Merge branch '{branch}'");
    git_at(dir, &at(), &["merge", "-q", "--no-ff", "-m", &msg, branch]);
}

/// main: a, then a branch `side` with b merged back as m, then c on main (HEAD).
fn repo(root: &Path) -> std::path::PathBuf {
    let p = root.join("repo");
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    commit(&p, "a");
    git(&p, &["checkout", "-q", "-b", "side"]);
    commit(&p, "b on side");
    git(&p, &["checkout", "-q", "main"]);
    commit(&p, "a2");
    merge(&p, "side");
    commit(&p, "c");
    git(&p, &["tag", "v1"]);
    p
}

fn cells(body: &str) -> usize {
    body.matches("class=\"graph\" aria-hidden=\"true\"").count()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_graph_view_draws_a_hidden_cell_per_row_and_marks_refs() {
    let root = tempfile::tempdir().expect("tmp");
    let router = gitterrace_web::app_router(repo(root.path())).expect("router");
    let (_, body) = page(&router, "/log/HEAD?view=graph").await;
    assert_eq!(cells(&body), 5, "{body}");
    assert!(
        body.contains("class=\"m main\""),
        "the merge is a hollow mainline dot"
    );
    assert!(
        body.contains("<span class=\"gref cur\">main</span>"),
        "{body}"
    );
    assert!(body.contains("<span class=\"gref\">v1</span>"), "{body}");
    assert!(body.contains("aria-current=\"page\">Graph</a>"), "{body}");
    assert!(
        !body.contains("class=\"day\""),
        "no day header cuts the lines"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_view_has_no_graph_and_a_filter_hides_the_switch() {
    let root = tempfile::tempdir().expect("tmp");
    let router = gitterrace_web::app_router(repo(root.path())).expect("router");
    let (_, list) = page(&router, "/log/HEAD").await;
    assert_eq!(cells(&list), 0);
    assert!(!list.contains("gref"));
    assert!(list.contains("aria-current=\"page\">List</a>"));
    let (_, filtered) = page(&router, "/log/HEAD?path=x&view=graph").await;
    assert_eq!(cells(&filtered), 0);
    assert!(!filtered.contains(">Graph</a>"));
}

#[tokio::test(flavor = "multi_thread")]
async fn lines_leaving_one_page_enter_the_next() {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    commit(&p, "base");
    git(&p, &["checkout", "-q", "-b", "long"]);
    commit(&p, "side work");
    git(&p, &["checkout", "-q", "main"]);
    // 60 mainline commits: the branch line stays open across the page edge at 50.
    for i in 0..60 {
        commit(&p, &format!("main {i}"));
    }
    merge(&p, "long");
    let router = gitterrace_web::app_router(&p).expect("router");
    let (_, one) = page(&router, "/log/HEAD?view=graph").await;
    let (_, two) = page(&router, "/log/HEAD?view=graph&page=1").await;
    assert!(one.contains("page=1&amp;view=graph") || one.contains("page=1&#38;view=graph"));
    let last = one.rsplit("class=\"graph\"").next().expect("cell");
    let first = two.split("class=\"graph\"").nth(1).expect("cell");
    // Two lines leave the bottom of page one (mainline and `long`); both enter page two.
    assert!(last.contains("M18 50V100"), "{last}");
    assert!(first.contains("M18 0V50"), "{first}");
}

/// main and three unmerged branches `l1`..`l3` off `a`, plus a linked worktree detached at
/// a commit of its own. No tip but main's is reachable from HEAD.
fn lanes(root: &Path) -> std::path::PathBuf {
    let p = root.join("lanes");
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    commit(&p, "a");
    for b in ["l1", "l2", "l3"] {
        git(&p, &["checkout", "-q", "-b", b, "main"]);
        commit(&p, &format!("tip of {b}"));
    }
    git(&p, &["checkout", "-q", "main"]);
    commit(&p, "tip of main");
    let wt = root.join("wt");
    let wt_s = wt.to_str().expect("utf-8");
    git(&p, &["worktree", "add", "-q", "--detach", wt_s, "main"]);
    commit(&wt, "tip of wt");
    p
}

#[tokio::test(flavor = "multi_thread")]
async fn the_graph_draws_every_branch_and_worktree_lane_with_its_label() {
    let root = tempfile::tempdir().expect("tmp");
    let router = gitterrace_web::app_router(lanes(root.path())).expect("router");
    let (_, all) = page(&router, "/log/HEAD?view=graph").await;
    for t in ["main", "l1", "l2", "l3", "wt"] {
        assert!(all.contains(&format!("tip of {t}")), "{t}: {all}");
    }
    assert_eq!(cells(&all), 6, "{all}");
    for b in ["l1", "l2", "l3"] {
        assert!(
            all.contains(&format!("<span class=\"gref\">{b}</span>")),
            "{all}"
        );
    }
    assert!(
        all.contains("<span class=\"gref cur\">main</span>"),
        "{all}"
    );
    assert!(
        all.contains("<span class=\"gref\">worktree wt</span>"),
        "{all}"
    );
    let (_, one) = page(&router, "/log/HEAD?view=graph&lanes=one").await;
    assert_eq!(cells(&one), 2, "{one}");
    assert!(
        !one.contains("tip of l1") && !one.contains("tip of wt"),
        "{one}"
    );
    assert!(one.contains("aria-current=\"page\">This ref</a>"), "{one}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_server_leaves_worktree_lanes_out() {
    let root = tempfile::tempdir().expect("tmp");
    let p = lanes(root.path());
    let mut cfg = gitterrace_config::Config::default();
    cfg.server.is_public = true;
    let router = gitterrace_web::app_router_with(&p, cfg).expect("router");
    let (_, all) = page(&router, "/log/HEAD?view=graph").await;
    assert!(all.contains("tip of l3"), "{all}");
    assert!(
        !all.contains("tip of wt") && !all.contains("worktree wt"),
        "{all}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lanes_of_many_branches_run_on_across_pages() {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    commit(&p, "base");
    git(&p, &["tag", "base"]);
    for i in 0..60 {
        commit(&p, &format!("main {i}"));
    }
    // Four tips newer than all of main, each with a line down to `base` on page two.
    for b in ["b1", "b2", "b3", "b4"] {
        git(&p, &["checkout", "-q", "-b", b, "base"]);
        commit(&p, &format!("tip of {b}"));
    }
    git(&p, &["checkout", "-q", "main"]);
    let router = gitterrace_web::app_router(&p).expect("router");
    let (_, one) = page(&router, "/log/HEAD?view=graph").await;
    let (_, two) = page(&router, "/log/HEAD?view=graph&page=1").await;
    assert_eq!(cells(&one), 50);
    for b in ["b1", "b2", "b3", "b4"] {
        assert!(
            one.contains(&format!("<span class=\"gref\">{b}</span>")),
            "{b}"
        );
    }
    // Mainline plus four branch lines leave page one and enter page two.
    let last = one.rsplit("class=\"graph\"").next().expect("cell");
    let first = two.split("class=\"graph\"").nth(1).expect("cell");
    assert!(last.matches(" 50V100").count() >= 5, "{last}");
    assert!(first.matches(" 0V50").count() >= 5, "{first}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wide_history_folds_past_three_columns() {
    let Ok(repo) = std::env::var("TEST_GRAPH_REPO") else {
        return;
    };
    let router = gitterrace_web::app_router(Path::new(&repo)).expect("router");
    // x of column c is 12c + 6; a path's coordinates alternate x and y.
    let num = regex::Regex::new(r"[MC ]?(\d+) (\d+)").expect("re");
    let mut folded = 0;
    for pg in 0..3 {
        let (_, body) = page(&router, &format!("/log/HEAD?view=graph&page={pg}")).await;
        for c in body.split("class=\"graph\"").skip(1) {
            let c = c.split("</span></li>").next().unwrap_or_default();
            let lines = c.split("class=\"gd\"").next().unwrap_or_default();
            let lines = lines.split("none\">").nth(1).unwrap_or_default();
            let dot = c.split("cx=\"").nth(1).and_then(|s| s.split('"').next());
            let xs = num
                .captures_iter(lines)
                .filter_map(|m| m[1].parse::<u32>().ok())
                .chain(dot.and_then(|d| d.parse().ok()));
            let cols = xs.map(|x| (x - 6) / 12 + 1).max().unwrap_or(1);
            let count = |cls: &str| -> u32 {
                c.split(&format!("class=\"gfold {cls}\">+"))
                    .nth(1)
                    .and_then(|s| s.split('<').next())
                    .map_or(0, |n| n.parse().expect("count"))
            };
            assert_eq!(count("n"), cols.saturating_sub(3), "page {pg}: {c}");
            assert_eq!(count("w"), cols.saturating_sub(8), "page {pg}: {c}");
            folded += usize::from(count("n") > 0);
        }
    }
    assert!(folded > 0, "a wide history folds some rows");
}
