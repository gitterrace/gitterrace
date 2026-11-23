//! The settings page wears the same top bar as every repository page.

use std::path::Path;

use crate::app::Env;
use crate::settings::tests::{page, repo};

/// A server over two repositories, so the repository switcher shows. The repositories open in
/// the background: this returns once both are served, so no page races the second one.
async fn two_repos(root: &Path, dir: &Path) -> axum::Router {
    let second = root.join("s");
    let built = gitterrace_testkit::Fixture::init(&second).write("b.txt", "b\n").commit("two");
    let mut env = Env::new(gitterrace_config::Config::default());
    env.settings_dir = Some(dir.to_path_buf());
    let router = crate::build_server(
        vec![repo(root), built.path().to_path_buf()],
        None,
        None,
        &env,
        None,
        None,
    )
    .expect("router")
    .0;
    gitterrace_testkit::wait::until("both repositories open", || async { env.served() == 2 }).await;
    router
}

/// Every page kind carries the shared top bar: brand, repository switcher with several
/// repositories, theme button and tab row; repository pages add the branch switcher, the
/// server's own page leaves out only what needs a repository. A page with a header of its own
/// fails here.
#[tokio::test]
async fn every_page_kind_renders_the_shared_header() {
    let (root, dir) = (tempfile::tempdir().expect("t"), tempfile::tempdir().expect("t"));
    let app = two_repos(root.path(), dir.path()).await;
    for (uri, repo_page) in [
        ("/r/r/", true),
        ("/r/r/log/HEAD", true),
        ("/r/r/tree/HEAD/", true),
        ("/_settings", false),
        ("/_settings?f=actions", false),
    ] {
        let (s, html) = page(&app, uri).await;
        if uri.contains("f=actions") && s == 404 {
            continue;
        }
        assert_eq!(s, 200, "{uri}");
        let must = [
            "class=\"brand\"",
            "id=\"reposwitch\"",
            "id=\"repobtn\"",
            "id=\"theme\"",
            "<nav class=\"tabnav\">",
            "class=\"skip\"",
        ];
        for part in must {
            assert!(html.contains(part), "{uri}: the shared header lacks {part}");
        }
        assert!(
            !html.contains("id=\"reposwitch\" hidden"),
            "{uri}: the switcher is hidden with two repositories"
        );
        assert_eq!(html.matches("<header class=\"top\"").count(), 1, "{uri}");
        assert!(html.contains("class=\"topsearch\""), "{uri}: every page can search");
        assert_eq!(
            html.contains("id=\"refbtn\""),
            repo_page,
            "{uri}: the branch switcher follows the repository"
        );
        assert!(!html.contains("class=\"bare\""), "{uri}");
    }
    let (_, html) = page(&app, "/_settings").await;
    assert!(
        html.contains("<a href=\"/_settings\" class=\"on\" aria-current=\"page\">Settings</a>"),
        "Settings is the current tab"
    );
    assert!(html.contains("name=\"gitterrace-settings\""), "settings meta stays");
    assert!(html.contains("/static/settings.css"), "settings.css stays");
}

#[tokio::test]
async fn the_settings_page_searches_the_first_served_repository() {
    let (root, dir) = (tempfile::tempdir().expect("t"), tempfile::tempdir().expect("t"));
    let app = two_repos(root.path(), dir.path()).await;
    let (_, html) = page(&app, "/_settings").await;
    assert!(html.contains("class=\"topsearch\" action=\"/r/"), "{html}");
    assert!(html.contains("/search\""));
    assert!(html.contains("id=\"searchbtn\""));
}

#[tokio::test]
async fn every_page_carries_the_release_for_the_menu() {
    let (root, dir) = (tempfile::tempdir().expect("t"), tempfile::tempdir().expect("t"));
    let app = two_repos(root.path(), dir.path()).await;
    let want = format!("name=\"gt-release\" content=\"{}\"", env!("CARGO_PKG_VERSION"));
    for uri in ["/r/r/", "/_settings"] {
        let (_, html) = page(&app, uri).await;
        assert!(html.contains(&want), "{uri}: no release meta");
    }
}
