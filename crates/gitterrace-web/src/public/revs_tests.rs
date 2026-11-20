//! A public server shows a full commit id only when a branch, tag, remote-tracking ref or
//! HEAD reaches it: a stash or a dangling commit stays dark through every page, redirect and
//! API route that takes a revision, while reachable history stays readable by its id.

use axum::{Router, body::Body, http::Request};
use gitterrace_config::{Feature, parse};
use gitterrace_testkit::{Fixture, git};
use http_body_util::BodyExt;
use tower::ServiceExt;

const HOST: &str = "git.example.com";
/// Committed only in the stash and the dangling commit.
const SECRET: &str = "stash-only-5be1";
/// Committed on `main` in its first commit, gone from the tip.
const OLD: &str = "old-public-77ad";

struct Site {
    router: Router,
    repo_id: String,
    old: String,
    hidden: Vec<String>,
    _dir: tempfile::TempDir,
}

/// `main` whose first commit holds [`OLD`], a stash and a dangling commit holding
/// [`SECRET`], served publicly with every feature and the API on.
fn site() -> Site {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path().join("repo");
    let f = Fixture::init(&p).write("a.txt", &format!("{OLD}\n")).commit("feat: first");
    let old = f.git(&["rev-parse", "HEAD"]);
    let f = f.write("a.txt", "tip\n").commit("fix: second");
    let tip = f.git(&["rev-parse", "HEAD"]);
    let f = f.write("a.txt", &format!("{SECRET}\n")).stash("wip");
    let stash = f.git(&["rev-parse", "stash"]);
    let index = f.git(&["rev-parse", "stash^2"]);
    let tree = f.git(&["rev-parse", "stash:"]);
    let dangling = git(&p, &["commit-tree", &tree, "-p", &tip, "-m", "dangling"]);
    let repo_id = gitterrace_git::Repo::discover(&p).expect("repo").id().expect("id");
    let on: String = Feature::ALL.map(|f| [f.key(), " = true\n"].concat()).concat();
    let text = format!("[server]\nis_public = true\nhosts = [\"{HOST}\"]\n[features]\n{on}");
    let router = crate::app_router_with(&p, parse(&text).expect("parse").config).expect("router");
    Site { router, repo_id, old, hidden: vec![stash, index, dangling], _dir: dir }
}

async fn send(r: &Router, uri: &str) -> (u16, String) {
    let req = Request::get(uri).header("host", HOST).body(Body::empty()).expect("req");
    let res = r.clone().oneshot(req).await.expect("res");
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Every public route that takes a revision, with `rev` in its revision slot, and whether a
/// readable revision answers it with 200.
fn routes(rev: &str, repo_id: &str) -> Vec<(String, bool)> {
    let r = "/r/repo";
    vec![
        (format!("{r}/blob/{rev}/a.txt"), true),
        (format!("{r}/raw/{rev}/a.txt"), true),
        (format!("{r}/hl/{rev}/a.txt"), true),
        (format!("{r}/blame/{rev}/a.txt"), true),
        (format!("{r}/tree/{rev}"), true),
        (format!("{r}/commit/{rev}"), true),
        (format!("{r}/diff/{rev}/0"), false),
        (format!("{r}/log/{rev}"), true),
        (format!("{r}/compare?base=HEAD&head={rev}"), false),
        (format!("{r}/compare?base={rev}&head=HEAD"), false),
        (format!("{r}/search?q=only-5be&rev={rev}"), false),
        (format!("{r}/search?q=public-77&rev={rev}"), false),
        (format!("{r}/api/files/{rev}"), false),
        (format!("{r}/api/last/{rev}/a.txt"), false),
        (format!("{r}/api/v1/blob?rev={rev}&path=a.txt"), true),
        (format!("{r}/api/v1/tree?rev={rev}"), true),
        (format!("{r}/api/v1/commits/{rev}"), true),
        (format!("{r}/api/v1/log?rev={rev}"), true),
        (format!("/go/{repo_id}/{rev}/a.txt"), false),
        (format!("/go/{repo_id}/{rev}"), false),
    ]
}

#[tokio::test]
async fn a_public_server_hides_a_stash_or_dangling_commit_on_every_revision_route() {
    let s = site();
    let unknown = "0".repeat(40);
    for id in &s.hidden {
        for rev in [id.clone(), format!("{id}^0"), format!("{id}~0")] {
            let probes = routes(&rev, &s.repo_id);
            let baseline = routes(&unknown, &s.repo_id);
            for ((uri, _), (unknown_uri, _)) in probes.into_iter().zip(baseline) {
                let (status, body) = send(&s.router, &uri).await;
                assert!(!body.contains(SECRET), "{uri} showed the secret ({status})");
                let (want, _) = send(&s.router, &unknown_uri).await;
                assert_eq!(status, want, "{uri} answers unlike an unknown id");
                assert_ne!(status, 302, "{uri} redirected");
                if !uri.contains("/search?") && !uri.contains("/compare?") {
                    assert_ne!(status, 200, "{uri} answered");
                }
            }
        }
    }
}

#[tokio::test]
async fn a_public_server_still_reads_reachable_history_by_its_full_id() {
    let s = site();
    for (uri, ok) in routes(&s.old, &s.repo_id) {
        let (status, body) = send(&s.router, &uri).await;
        if uri.starts_with("/go/") {
            assert_eq!(status, 302, "{uri}");
        } else if ok {
            assert_eq!(status, 200, "{uri}: {body}");
        }
        assert!(!body.contains(SECRET), "{uri} showed the secret");
    }
    let (status, body) = send(&s.router, &format!("/r/repo/raw/{}/a.txt", s.old)).await;
    assert_eq!((status, body.trim()), (200, OLD));
    let parent = format!("{}^", s.hidden[0]);
    let (status, body) = send(&s.router, &format!("/r/repo/raw/{parent}/a.txt")).await;
    assert_eq!((status, body.trim()), (200, "tip"), "a stash's public parent");
}
