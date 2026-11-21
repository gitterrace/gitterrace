//! Each `[limits]` and `[server]` knob that the web layer reads changes what a page shows.

use gitterrace_config::Config;
use gitterrace_testkit::{Fixture, page as get};

use crate::wiring_tests::{repo, router, with};

#[tokio::test]
async fn limits_search_file_lines_caps_lines_per_file() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let mid = |b: &str| {
        b.split("mid.txt</a><span class=\"tag\">")
            .nth(1)
            .and_then(|r| r.split(" match").next())
            .map(str::to_owned)
    };
    let (_, all) = get(&router(&p, Config::default()), "/search?q=needle").await;
    assert_eq!(mid(&all).as_deref(), Some("100"), "the default keeps 100 of 450");
    let capped = with(|c| c.limits.search_file_lines = 3);
    let (_, few) = get(&router(&p, capped), "/search?q=needle").await;
    assert_eq!(mid(&few).as_deref(), Some("3"));
}

#[tokio::test]
async fn limits_search_commits_caps_the_commit_list() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let url = "/search?q=.&regex=1&kind=commits";
    let (_, all) = get(&router(&p, Config::default()), url).await;
    assert!(!all.contains("Showing the first results only"), "{all}");
    let (_, one) = get(&router(&p, with(|c| c.limits.search_commits = 1)), url).await;
    assert!(one.contains("Showing the first results only"), "{one}");
}

#[tokio::test]
async fn limits_blob_lines_cuts_the_file_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, all) = get(&router(&p, Config::default()), "/blob/HEAD/mid.txt").await;
    assert!(!all.contains("Truncated after"));
    let (_, cut) = get(&router(&p, with(|c| c.limits.blob_lines = 10)), "/blob/HEAD/mid.txt").await;
    assert!(cut.contains("Truncated after 10 lines."), "{cut}");
    assert!(!cut.contains("id=\"L11\""));
}

#[tokio::test]
async fn limits_highlight_kb_shows_larger_text_plain() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = Fixture::init(&dir.path().join("r"))
        .write("a.rs", &"fn f() { let x = 1; }\n".repeat(100))
        .commit("feat: rust")
        .path()
        .to_owned();
    let (_, lit) = get(&router(&p, Config::default()), "/blob/HEAD/a.rs").await;
    assert!(lit.contains("class=\"s-"), "highlighted by default");
    let (_, plain) = get(&router(&p, with(|c| c.limits.highlight_kb = 1)), "/blob/HEAD/a.rs").await;
    assert!(!plain.contains("class=\"s-"), "2.2 KiB over a 1 KiB limit shows plain");
}

#[tokio::test]
async fn limits_log_pages_clamps_the_page_number() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let (_, far) = get(&router(&p, Config::default()), "/log/HEAD?page=5").await;
    assert!(!far.contains("fix: second"), "page 5 of a 2-commit log is empty");
    let (_, clamped) = get(&router(&p, with(|c| c.limits.log_pages = 1)), "/log/HEAD?page=5").await;
    let (_, page1) = get(&router(&p, Config::default()), "/log/HEAD?page=1").await;
    assert_eq!(clamped.contains("fix: second"), page1.contains("fix: second"));
    assert_ne!(far, clamped, "the clamp moved page 5 to page 1");
}

#[tokio::test]
async fn limits_preview_mb_stops_reading_media_facts() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend([0, 0, 0, 7, 0, 0, 0, 5, 8, 6, 0, 0, 0]);
    png.extend(vec![0u8; 1 << 20]);
    let root = dir.path().join("r");
    std::fs::create_dir_all(&root).expect("mkdir");
    let f = Fixture::init(&root);
    std::fs::write(root.join("a.png"), &png).expect("write");
    let p = f.commit("feat: image").path().to_owned();
    let (_, facts) = get(&router(&p, Config::default()), "/blob/HEAD/a.png").await;
    assert!(facts.contains("7 × 5") || facts.contains("7×5") || facts.contains("7 x 5"), "{facts}");
    let (_, none) = get(&router(&p, with(|c| c.limits.preview_mb = 1)), "/blob/HEAD/a.png").await;
    assert_ne!(facts, none, "a 1 MiB image over a 1 MB limit shows no pixel size");
}

#[tokio::test]
async fn limits_worktree_files_reach_the_uncommitted_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    for i in 0..3 {
        std::fs::write(p.join(format!("new{i}.txt")), "x\n").expect("write");
    }
    let (_, all) = get(&router(&p, Config::default()), "/worktree").await;
    let (_, one) = get(&router(&p, with(|c| c.limits.worktree_files = 1)), "/worktree").await;
    let n = |b: &str| b.matches("class=\"file\"").count();
    assert_eq!((n(&all), n(&one)), (3, 1));
}

#[tokio::test]
async fn limits_unindexed_commits_reach_the_index() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path(), "r");
    let cfg = with(|c| c.limits.unindexed_commits = 7);
    let a = crate::wiring_tests::app(&p, Config::default());
    assert_eq!(a.index.fresh_cap(), 2000);
    let env = crate::app::Env::new(cfg);
    let (_router, _tasks, _app) =
        crate::repo_router_tasks(&p, String::new(), "r".into(), None, &env).expect("router");
    let built = env.apps();
    assert_eq!(built.first().map(|a| a.index.fresh_cap()), Some(7));
}

#[tokio::test]
async fn server_public_timeout_secs_reaches_the_gate() {
    let g = crate::guard::Gate::new(None, &with(|c| c.server.public_timeout_secs = 5));
    assert_eq!(g.timeout_for_test(), std::time::Duration::from_secs(5));
}

/// A public server warns for each raised cost key and stays quiet for a lowered one or for a
/// loopback server with the same values.
#[test]
fn public_server_warns_about_raised_cost_keys() {
    let mut c = Config::default();
    c.api.diff_lines = 40_000;
    c.limits.search_matches = 10;
    assert!(crate::public_cost_warnings(&c).is_empty(), "not public");
    c.server.is_public = true;
    let w = crate::public_cost_warnings(&c);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("api.diff_lines"), "{w:?}");
}

/// A changed image's diff header names pixels and bytes, not `+0 -0`.
#[tokio::test]
async fn a_changed_image_header_says_its_size() {
    let dir = tempfile::tempdir().expect("tmp");
    let png = |w: u8, h: u8, pad: usize| {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend([0, 0, 0, w, 0, 0, 0, h, 8, 6, 0, 0, 0]);
        b.extend(vec![0u8; pad]);
        b
    };
    let root = dir.path().join("r");
    std::fs::create_dir_all(&root).expect("mkdir");
    let f = Fixture::init(&root);
    std::fs::write(root.join("a.png"), png(7, 5, 10)).expect("write");
    let f = f.commit("feat: image");
    std::fs::write(root.join("a.png"), png(7, 5, 2000)).expect("write");
    let p = f.commit("fix: heavier image").path().to_owned();
    let (_, page) = get(&router(&p, Config::default()), "/commit/HEAD").await;
    let at = page.find("data-path=\"a.png\"").expect("a.png section");
    let head = &page[at..at + page[at..].find("</header>").expect("header")];
    assert!(head.contains(">7 x 5, 39 B to 2.0 KiB<"), "{head}");
    assert!(!head.contains("+0"), "{head}");
}

#[tokio::test]
async fn an_unknown_binary_names_its_format_size_and_a_download() {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("r");
    let f = Fixture::init(&root);
    let mut db = b"SQLite format 3\0".to_vec();
    db.resize(4096, 0);
    std::fs::write(root.join("app.data"), &db).expect("write");
    let p = f.commit("feat: database").path().to_owned();
    let (_, page) = get(&router(&p, Config::default()), "/blob/HEAD/app.data").await;
    assert!(page.contains("SQLite database \u{b7} 4.0 KiB"), "{page}");
    assert!(page.contains("/raw/HEAD/app.data\" download>Download</a>"), "{page}");
}

#[tokio::test]
async fn a_folder_offers_a_search_scoped_to_itself() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = Fixture::init(&dir.path().join("r"))
        .write("src/a/x.txt", "needle in a\n")
        .write("src/b/y.txt", "needle in b\n")
        .commit("feat: two folders")
        .path()
        .to_owned();
    let app = router(&p, Config::default());
    let (_, root) = get(&app, "/tree/HEAD").await;
    assert!(!root.contains(">Search here</a>"), "the root has the header search");
    let (_, folder) = get(&app, "/tree/HEAD/src/a").await;
    let href = folder
        .split("\">Search here</a>")
        .next()
        .and_then(|s| s.rsplit("href=\"").next())
        .filter(|_| folder.contains(">Search here</a>"))
        .expect("a Search here action")
        .replace("&#38;", "&")
        .replace("&amp;", "&");
    // Links carry the repo base (`/r/<name>`), and the test router serves the repo at `/`.
    let href = href[href.find("/search").expect("a search link")..].to_owned();
    assert!(href.ends_with("glob=src%2Fa%2F"), "{href}");
    let (_, hits) = get(&app, &format!("{href}&q=needle")).await;
    assert!(hits.contains("src/a/x.txt"), "{hits}");
    assert!(!hits.contains("src/b/y.txt"), "only the folder: {hits}");
    let (_, form) = get(&app, &href).await;
    assert!(form.contains("name=\"glob\" value=\"src/a/\""), "{form}");
}
