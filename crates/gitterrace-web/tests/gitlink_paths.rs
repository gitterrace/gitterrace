//! A path that was a submodule at some commit never makes a 5xx: the pages answer a 4xx or
//! 200 for the gitlink and for anything below it, and the last commit per entry names the
//! commit that really touched each entry, with the gitlink's commit as the old submodule bump.

use gitterrace_testkit::{git, git_at, oracle, page};

const GONE: &str = "1dfaa4db2479753a46a3e90f2c3c89d89d0b21f1";
const D: &str = "2024-01-01T00:00:00+00:00";

fn head_of(dir: &std::path::Path, rev: &str) -> String {
    git(dir, &["rev-parse", rev])
}

#[tokio::test(flavor = "multi_thread")]
async fn a_former_submodule_path_answers_below_5xx_and_last_commits_are_exact() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    git_at(p, D, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "a\n").expect("write");
    git_at(p, D, &["add", "a.txt"]);
    let info = format!("160000,{GONE},sub");
    git_at(p, D, &["update-index", "--add", "--cacheinfo", &info]);
    git_at(p, "2024-01-01T01:00:00+00:00", &["commit", "-q", "-m", "submodule"]);
    let with_gitlink = head_of(p, "HEAD");
    git_at(p, D, &["rm", "-q", "--cached", "sub"]);
    std::fs::create_dir_all(p.join("sub")).expect("mkdir");
    std::fs::write(p.join("sub/f"), "x\n").expect("write");
    git_at(p, D, &["add", "sub"]);
    git_at(p, "2024-01-02T01:00:00+00:00", &["commit", "-q", "-m", "vendored"]);
    let vendored = head_of(p, "HEAD");
    std::fs::write(p.join("b.txt"), "b\n").expect("write");
    git_at(p, D, &["add", "b.txt"]);
    git_at(p, "2024-01-03T01:00:00+00:00", &["commit", "-q", "-m", "more"]);

    let router = gitterrace_web::app_router(p).expect("router");
    oracle::ready(&router).await;

    for uri in [
        format!("/tree/{with_gitlink}"),
        format!("/tree/{with_gitlink}/sub"),
        format!("/tree/{with_gitlink}/sub/f"),
        format!("/blob/{with_gitlink}/sub"),
        format!("/blob/{with_gitlink}/sub/f"),
        format!("/api/last/{with_gitlink}"),
        format!("/api/last/{with_gitlink}/sub"),
        format!("/api/last/{with_gitlink}/sub/f"),
        "/tree/HEAD/sub".to_string(),
        "/blob/HEAD/sub/f".to_string(),
    ] {
        let (status, body) = page(&router, &uri).await;
        assert!(status < 500, "{uri}: {status} {body}");
    }

    let (status, body) = page(&router, "/api/last/HEAD").await;
    assert_eq!(status, 200, "{body}");
    let list: serde_json::Value = serde_json::from_str(&body).expect("json");
    let id_of = |name: &str| {
        list.as_array()
            .and_then(|a| a.iter().find(|e| e["name"] == name))
            .and_then(|e| e["id"].as_str())
            .map(str::to_owned)
    };
    let first = head_of(p, &with_gitlink);
    assert_eq!(id_of("a.txt"), Some(first), "a.txt was never touched again");
    assert_eq!(id_of("sub"), Some(vendored), "sub changed when it was vendored");
    assert_eq!(id_of("b.txt"), Some(head_of(p, "HEAD")));
}
