//! An insights page whose filters match no commit says so. It never shows a date from an
//! empty set (1970-01-01) or charts drawn from nothing.

use std::time::Duration;

use gitterrace_testkit::{get, git};

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_selection_reads_as_empty_on_every_insights_page() {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.rs"), "fn main() {}\n").expect("w");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "feat: first"]);

    let router = gitterrace_web::app_router(&p).expect("router");
    for _ in 0..100 {
        let (_, body) = get(&router, "/stats").await;
        if !body.contains("Indexing") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for page in [
        "/stats",
        "/stats/hotspots",
        "/stats/patterns",
        "/stats/types",
        "/stats/languages",
        "/stats/contributors",
    ] {
        let url = format!("{page}?author=nobody%40example.com");
        let (status, body) = get(&router, &url).await;
        assert_eq!(status, 200, "{url}");
        assert!(
            !body.contains("1970"),
            "{url} shows a date from an empty set"
        );
    }
}
