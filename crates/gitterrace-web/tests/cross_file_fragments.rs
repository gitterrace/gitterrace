//! A link to a fragment of another file keeps the plain fragment, and so does a same-page link:
//! heading ids are the plain slug, so `#intro` lands without any script.

use gitterrace_testkit::{git, git_at, page};

#[test]
fn cross_file_fragments_stay_plain() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path();
        let d = "2024-01-01T00:00:00+00:00";
        git_at(p, d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(p.join("docs")).expect("mkdir");
        std::fs::write(
            p.join("docs/guide.md"),
            "# Intro\n\n[a](architecture.md#syntax-highlighting) [b](../other.md#top) \
             [c](./sub/page.md#a-b) [d](#intro)\n",
        )
        .expect("write");
        git_at(p, d, &["add", "docs"]);
        git_at(p, d, &["commit", "-q", "-m", "docs"]);
        let sha = git(p, &["rev-parse", "HEAD"]);

        let router = gitterrace_web::app_router(p).expect("router");
        let (status, body) = page(&router, &format!("/blob/{sha}/docs/guide.md")).await;
        assert_eq!(status, 200, "{body}");
        for want in [
            "/blob/SHA/docs/architecture.md#syntax-highlighting",
            "/blob/SHA/other.md#top",
            "/blob/SHA/docs/sub/page.md#a-b",
        ] {
            let want = format!("{}\"", want.replace("SHA", &sha));
            assert!(body.contains(&want), "{want} missing in {body}");
        }
        assert!(
            body.contains("href=\"#intro\"") && body.contains("id=\"intro\""),
            "same-page link and its heading stay plain"
        );
        assert!(
            !body.contains(
                "href=\"/blob/SHA/other.md#user-content-"
                    .replace("SHA", &sha)
                    .as_str()
            ) && !body.contains("#user-content-top\""),
            "cross-file fragment was prefixed"
        );
        assert!(
            !body.contains("other.md#user-content-"),
            "cross-file fragment was prefixed"
        );
        assert!(
            !body.contains("page.md#user-content-"),
            "cross-file fragment was prefixed"
        );
    });
}
