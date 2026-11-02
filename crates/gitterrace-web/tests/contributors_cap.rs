//! The contributors page renders the top rows and links to the full list; its KPI and the API
//! still count everyone.

use std::time::Duration;

use gitterrace_testkit::{get, git};

#[tokio::test(flavor = "multi_thread")]
async fn contributors_page_caps_rows_and_links_to_all() {
    let root = tempfile::tempdir().expect("tmp");
    let path = root.path();
    git(path, &["init", "-q", "-b", "main"]);
    for i in 0..105 {
        std::fs::write(path.join("f.txt"), format!("{i}\n")).expect("write");
        git(path, &["add", "."]);
        let (name, email) = (format!("user.name=P{i}"), format!("user.email=p{i}@e.x"));
        git(
            path,
            &["-c", &name, "-c", &email, "commit", "-q", "-m", "feat: x"],
        );
    }
    let router = gitterrace_web::app_router(path).expect("router");
    let mut page = String::new();
    for _ in 0..200 {
        page = get(&router, "/stats/contributors").await.1;
        if !page.contains("data-progress") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let rows = |html: &str| html.matches("class=\"av ").count();
    assert_eq!(rows(&page), 100, "the page renders the top rows only");
    assert!(page.contains("Show all 105 contributors"));
    assert!(page.contains("all=1"));
    assert!(page.contains(">105<"), "the KPI counts everyone");
    let (_, all) = get(&router, "/stats/contributors?all=1").await;
    assert_eq!(rows(&all), 105);
    assert!(!all.contains("Show all"));
}
