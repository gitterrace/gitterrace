//! With a non-default branch selected, every repo nav link, the repo name link and the insights
//! tabs keep that branch, whether the page took it from its path or from `?rev=`.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{git, page};

fn repo(dir: &std::path::Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a\n").expect("write");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "feat: a"]);
    git(dir, &["branch", "dev"]);
}

/// The `href`s of the repo name link and of every link in the top nav and the insights tabs.
fn nav_links(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut take = |chunk: &str| {
        for h in chunk.split("href=\"").skip(1) {
            out.push(h.split('"').next().unwrap_or_default().to_owned());
        }
    };
    if let Some(r) = body.split("id=\"repolink\" ").nth(1) {
        take(&format!("x{}", r.split('>').next().unwrap_or_default()));
    }
    for open in ["<nav class=\"tabnav\">", "<nav class=\"tabs\">"] {
        for r in body.split(open).skip(1) {
            take(&format!(
                "x{}",
                r.split("</nav>").next().unwrap_or_default()
            ));
        }
    }
    out
}

#[test]
fn every_nav_link_keeps_the_selected_branch() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        repo(dir.path());
        let router = gitterrace_web::app_router(dir.path()).expect("router");
        // `dev` and HEAD name one commit: a memo the HEAD page filled first must not answer
        // `?rev=dev`, so the warm-up's page is in place before the branch is asked for.
        for _ in 0..250 {
            let (_, body) = page(&router, "/stats").await;
            if !body.contains("Indexing history") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        for url in [
            "/?rev=dev",
            "/tree/dev",
            "/log/dev",
            "/refs?rev=dev",
            "/stats?rev=dev",
        ] {
            let (status, body) = page(&router, url).await;
            assert_eq!(status, 200, "{url}");
            let links = nav_links(&body);
            assert!(links.len() >= 4, "{url}: found only {links:?}");
            for l in links {
                assert!(
                    l.contains("rev=dev") || l.contains("/dev"),
                    "{url}: nav link {l} drops the selected branch"
                );
            }
        }
        let (_, body) = page(&router, "/").await;
        for l in nav_links(&body) {
            assert!(!l.contains("rev="), "the default branch adds no ?rev=: {l}");
        }
    });
}
