//! The contributors page and its API insight order by the `metric` parameter. Commit order
//! and added-lines order differ on purpose: Ann commits most, Bob adds most.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::time::Duration;

use gitterrace_testkit::{git, page as get};

async fn ready(router: &axum::Router, uri: &str) -> String {
    let mut body = String::new();
    for _ in 0..200 {
        body = get(router, uri).await.1;
        if !body.contains("data-progress") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    body
}

fn commit(path: &std::path::Path, who: &str, file: &str, body: &str) {
    std::fs::write(path.join(file), body).expect("write");
    git(path, &["add", "."]);
    let (n, e) = (format!("user.name={who}"), format!("user.email={who}@e.x"));
    git(path, &["-c", &n, "-c", &e, "commit", "-q", "-m", "feat: x"]);
}

fn router() -> (tempfile::TempDir, axum::Router) {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path();
    git(p, &["init", "-q", "-b", "main"]);
    for i in 0..3 {
        commit(p, "Ann", "a.txt", &format!("{i}\n"));
    }
    commit(p, "Bob", "b.txt", &"line\n".repeat(50));
    let r = gitterrace_web::app_router(p).expect("router");
    (root, r)
}

fn first_person(html: &str) -> &str {
    let at = html.find("class=\"who\"").expect("a row");
    let rest = &html[at..];
    let from = rest.find("<b>").expect("name") + 3;
    &rest[from..from + 3]
}

#[tokio::test(flavor = "multi_thread")]
async fn metric_added_sorts_by_added_lines() {
    let (_root, r) = router();
    let page = ready(&r, "/stats/contributors?metric=added").await;
    assert_eq!(first_person(&page), "Bob", "ordered by added lines");
    assert!(page.contains("class=\"tsb\""), "the two-sided bar shows");
    let net = ready(&r, "/stats/contributors?metric=net&period=3m&bots=1").await;
    assert_eq!(first_person(&net), "Bob", "works with other filters");
    assert!(
        net.contains("period=3m") && net.contains("metric=added"),
        "{net}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn default_is_commits_and_unchanged() {
    let (_root, r) = router();
    let page = ready(&r, "/stats/contributors").await;
    assert_eq!(first_person(&page), "Ann");
    assert!(!page.contains("tsb\""), "no bar by default");
    assert!(page.contains("metric=added"), "the switch shows by default");
    let junk = ready(&r, "/stats/contributors?metric=bogus").await;
    assert_eq!(page, junk, "an unknown metric is the default");
}

#[tokio::test(flavor = "multi_thread")]
async fn api_insight_takes_metric() {
    let (_root, r) = router();
    let _ = ready(&r, "/stats/contributors").await;
    let name = |uri: &'static str| {
        let r = r.clone();
        async move {
            let v: serde_json::Value = serde_json::from_str(&get(&r, uri).await.1).expect("json");
            v["data"]["contributors"][0]["name"]
                .as_str()
                .expect("name")
                .to_owned()
        }
    };
    assert_eq!(name("/api/v1/insights/contributors").await, "Ann");
    assert_eq!(
        name("/api/v1/insights/contributors?metric=added").await,
        "Bob"
    );
}
