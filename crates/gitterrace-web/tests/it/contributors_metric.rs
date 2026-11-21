//! The contributors page and its API insight order by the `metric` parameter. Commit order
//! and added-lines order differ on purpose: Ann commits most, Bob adds most.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, page as get};

async fn ready(router: &axum::Router, uri: &str) -> String {
    gitterrace_testkit::wait::page_until(router, uri, gitterrace_testkit::wait::indexed).await
}

/// Commits `body` as `file` under the author `who`.
fn commit(f: Fixture, who: &str, file: &str, body: &str) -> Fixture {
    let f = f.write(file, body);
    f.git(&["add", "-A"]);
    let (n, e) = (format!("user.name={who}"), format!("user.email={who}@e.x"));
    f.git(&["-c", &n, "-c", &e, "commit", "-q", "-m", "feat: x"]);
    f
}

fn router() -> (tempfile::TempDir, axum::Router) {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path();
    let mut f = Fixture::init(p);
    for i in 0..3 {
        f = commit(f, "Ann", "a.txt", &format!("{i}\n"));
    }
    commit(f, "Bob", "b.txt", &"line\n".repeat(50));
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
    assert!(net.contains("q=after%3A3m") && net.contains("metric=added"), "{net}");
}

#[tokio::test(flavor = "multi_thread")]
async fn default_is_commits_and_unchanged() {
    let (_root, r) = router();
    let page = ready(&r, "/stats/contributors").await;
    assert_eq!(first_person(&page), "Ann");
    assert!(!page.contains("tsb\""), "no bar by default");
    assert!(page.contains("metric=added"), "the switch shows by default");
    let junk = ready(&r, "/stats/contributors?metric=bogus").await;
    // The repo switcher's target repeats the query as typed, so compare without it.
    let plain = |p: &str| {
        p.split("data-switch=\"")
            .map(|s| s.split_once('"').map_or(s, |x| x.1).to_owned())
            .collect::<String>()
    };
    assert_eq!(plain(&page), plain(&junk), "an unknown metric is the default");
}

#[tokio::test(flavor = "multi_thread")]
async fn api_insight_takes_metric() {
    let (_root, r) = router();
    let _ = ready(&r, "/stats/contributors").await;
    let name = |uri: &'static str| {
        let r = r.clone();
        async move {
            let v: serde_json::Value = serde_json::from_str(&get(&r, uri).await.1).expect("json");
            v["data"]["contributors"][0]["name"].as_str().expect("name").to_owned()
        }
    };
    assert_eq!(name("/api/v1/insights/contributors").await, "Ann");
    assert_eq!(name("/api/v1/insights/contributors?metric=added").await, "Bob");
}
