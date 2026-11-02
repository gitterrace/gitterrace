//! The page policy has no `unsafe-inline` for styles, so no page may rely on a `style`
//! attribute: the browser would drop it. Renders the pages that once carried one.

use gitterrace_testkit::{get, git};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn pages_carry_no_style_attribute() {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    let long = "line\n".repeat(250);
    std::fs::write(p.join("long.txt"), long).expect("w");
    std::fs::write(p.join("a.rs"), "fn main() {}\n").expect("w");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "feat(core): first"]);
    std::fs::write(p.join("a.rs"), "fn main() {\n}\n").expect("w");
    git(&p, &["commit", "-qam", "fix: second"]);

    let router = gitterrace_web::app_router(&p).expect("router");
    for _ in 0..100 {
        let (_, body) = get(&router, "/search?q=first&kind=commits").await;
        if body.contains("class=\"crow\"") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for page in [
        "/",
        "/log/HEAD",
        "/blob/HEAD/long.txt",
        "/stats/types",
        "/stats/hotspots",
        "/stats/languages",
        "/stats/contributors",
    ] {
        let (_, body) = get(&router, page).await;
        assert!(!body.contains("style=\""), "{page} has an inline style");
    }
}
