use std::path::Path;

use axum::Router;

use super::{url_stem, valid_name};
use axum::response::IntoResponse;
use gitterrace_testkit::git;

#[test]
fn names() {
    for ok in ["tendir", "gitterrace-2", "a.b", "x_y"] {
        assert!(valid_name(ok), "{ok}");
    }
    for bad in ["", ".", "..", ".hidden", "a/b", "a b", "a%2e"] {
        assert!(!valid_name(bad), "{bad}");
        assert!(valid_name(&url_stem(bad)), "{bad} -> {}", url_stem(bad));
    }
    assert_eq!(url_stem(".tmpAbC"), "tmpabc");
    assert_eq!(url_stem("My Repo"), "my-repo");
    assert_eq!(url_stem(".."), "repo");
    assert_eq!(url_stem("git.git"), "git");
    assert_eq!(url_stem(".git"), "git");
}

#[test]
fn recent_is_memoised_by_head() {
    let dir = tempfile::Builder::new()
        .prefix("repo")
        .tempdir()
        .expect("tmp");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["commit", "-q", "--allow-empty", "-m", "one"]);
    let table = super::Table::new(
        vec![p.to_path_buf()],
        crate::app::Env::new(gitterrace_config::Config::default()),
        None,
    )
    .expect("table");
    let s = table.snapshot()[0].clone();
    let now = crate::fmt::now() + 60;
    let first = s.recent(now);
    assert!(
        std::sync::Arc::ptr_eq(&first, &s.recent(now)),
        "same HEAD reuses the read"
    );
    git(p, &["commit", "-q", "--allow-empty", "-m", "two"]);
    let after = s.recent(now);
    let total = |r: &super::Recent| r.weeks.iter().sum::<u32>();
    assert_eq!((total(&first), total(&after)), (1, 2));
    assert_eq!(after.last.as_ref().map(|c| c.subject.as_str()), Some("two"));
}

/// The line counts every commit in the window, not the newest few thousand: a busy
/// repository must not show an empty tail.
#[test]
fn weekly_counts_the_whole_window() {
    let now = 100 * super::WEEK;
    // 5000 commits evenly over the 26 weeks, far more than the old 2000-commit read.
    let step = super::WEEK * super::SPARK_WEEKS as i64 / 5000;
    let times: Vec<i64> = (0..5000).map(|i| now - i * step).collect();
    let weeks = super::weekly(&times, now);
    assert_eq!(weeks.len(), super::SPARK_WEEKS);
    assert!(weeks.iter().all(|n| *n > 0), "{weeks:?}");
    assert_eq!(weeks.iter().sum::<u32>(), 5000);
}

#[tokio::test]
async fn the_slash_redirect_keeps_the_query() {
    let r = super::slash("/r/a?period=1y&bots=1".parse().expect("uri"))
        .await
        .into_response();
    let to = r.headers().get("location").expect("location");
    assert_eq!(to, "/r/a/?period=1y&bots=1");
}

/// A repository that will not open answers the error page; a public server never names a
/// host folder in it, a local one shows the reason.
#[test]
fn a_repository_that_will_not_open_hides_host_paths_when_public() {
    let e = std::io::Error::other("cannot open /srv/private/repo/.git/gitterrace");
    for (public, shows) in [(true, false), (false, true)] {
        let mut cfg = gitterrace_config::Config::default();
        cfg.server.public = public;
        let table = super::Table::new(Vec::new(), crate::app::Env::new(cfg), None).expect("table");
        let r = table.open_failed("repo", &e);
        assert_eq!(r.status(), 500);
        let body = futures_body(r);
        assert!(body.contains("repo could not be opened"), "{body}");
        assert_eq!(
            body.contains("/srv/private"),
            shows,
            "public={public}: {body}"
        );
    }
}

fn futures_body(r: axum::response::Response) -> String {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("rt");
    let b = rt
        .block_on(http_body_util::BodyExt::collect(r.into_body()))
        .expect("body")
        .to_bytes();
    String::from_utf8_lossy(&b).into_owned()
}

/// The ways a repository reaches a server, as the CLI does it (`discover_repos`, then the
/// server, or `POST /_repos`): a working tree, a bare clone (`git.git`, named `git`), a
/// folder holding both, and a bare clone joining a running server. Each lists exactly its
/// repositories under the right names and builds its index with no page asked for first:
/// progress under its prefix reaches ready, and the API counts its commits.
#[tokio::test(flavor = "multi_thread")]
async fn every_start_shape_serves_and_indexes_its_repositories() {
    let dir = tempfile::tempdir().expect("tmp");
    let work = gitterrace_testkit::Fixture::standard(&dir.path().join("all/work"))
        .path()
        .to_owned();
    let bare = dir.path().join("all/git.git");
    let clone = |to: &Path| {
        let (from, to) = (work.to_str().expect("utf8"), to.to_str().expect("utf8"));
        git(dir.path(), &["clone", "-q", "--bare", from, to]);
    };
    clone(&bare);
    let joining = dir.path().join("jq.git");
    clone(&joining);
    for (start, want) in [
        (work.clone(), vec!["work"]),
        (bare.clone(), vec!["git"]),
        (dir.path().join("all"), vec!["git", "work"]),
    ] {
        let paths = crate::discover_repos(std::slice::from_ref(&start), 2, &[]);
        let router = crate::multi_router(paths).expect("router");
        assert_eq!(listed(&router).await, want, "{}", start.display());
        for name in &want {
            assert_indexed(&router, name).await;
        }
    }
    let token = gitterrace_actions::Token::generate().expect("token");
    let secret = token.as_str().to_owned();
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let (router, _) =
        crate::build_server(vec![work], None, None, &env, Some(token), None).expect("r");
    let root = joining.canonicalize().expect("canon");
    let req = axum::http::Request::post("/_repos")
        .header("host", gitterrace_testkit::on_port("localhost"))
        .header("content-type", "application/json")
        .header(crate::add::TOKEN_HEADER, secret)
        .body(axum::body::Body::from(
            serde_json::json!({ "path": root }).to_string(),
        ))
        .expect("req");
    let res = tower::ServiceExt::oneshot(router.clone(), req)
        .await
        .expect("res");
    assert_eq!(res.status(), 201, "join a bare clone");
    assert_eq!(listed(&router).await, ["work", "jq"]);
    assert_indexed(&router, "jq").await;
}

async fn listed(router: &Router) -> Vec<String> {
    let (_, list) = gitterrace_testkit::get(router, "/api/repos").await;
    serde_json::from_str::<Vec<serde_json::Value>>(&list)
        .expect("json")
        .iter()
        .map(|i| i["name"].as_str().expect("name").to_owned())
        .collect()
}

/// `name` reaches ready on its own and its API counts the fixture's two commits.
async fn assert_indexed(router: &Router, name: &str) {
    let mut last = String::new();
    for _ in 0..500 {
        last = gitterrace_testkit::get(router, &format!("/r/{name}/api/progress"))
            .await
            .1;
        if last.contains("\"ready\":true") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        last.contains("\"ready\":true"),
        "{name} never indexed: {last}"
    );
    let (s, repo) = gitterrace_testkit::get(router, &format!("/r/{name}/api/v1/repo")).await;
    let v: serde_json::Value = serde_json::from_str(&repo).expect("repo json");
    assert_eq!(
        (s, v["totals"]["commits"].as_u64()),
        (200, Some(2)),
        "{name}: {repo}"
    );
}
