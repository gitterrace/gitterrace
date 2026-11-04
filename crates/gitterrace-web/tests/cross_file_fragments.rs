//! A link to a fragment of another file keeps the plain fragment. Only same-page fragments get
//! the `user-content-` id prefix: the target page's own script maps a plain fragment to the
//! prefixed id (static/nav.js), so this contract must not drift.

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
            "# Top\n\n[a](architecture.md#syntax-highlighting) [b](../other.md#top) \
             [c](./sub/page.md#a-b) [d](#top)\n",
        )
        .expect("write");
        git_at(p, d, &["add", "docs"]);
        git_at(p, d, &["commit", "-q", "-m", "docs"]);
        let sha = git(p, &["rev-parse", "HEAD"]);

        let router = gitterrace_web::app_router(p).expect("router");
        let (status, body) = page(&router, &format!("/blob/{sha}/docs/guide.md")).await;
        assert_eq!(status, 200, "{body}");
        for want in [
            "/blob/{sha}/docs/architecture.md#syntax-highlighting",
            "/blob/{sha}/other.md#top",
            "/blob/{sha}/docs/sub/page.md#a-b",
        ] {
            let want = format!("{}\"", want.replace("{sha}", &sha));
            assert!(body.contains(&want), "{want} missing in {body}");
        }
        assert!(
            body.contains("href=\"#user-content-top\""),
            "same-page link keeps its prefix"
        );
        assert!(
            !body.contains("#user-content-syntax"),
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
