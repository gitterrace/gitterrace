//! Tests for several repositories on one server: links, names and the Host guard.

use std::path::PathBuf;

use gitterrace_git::Repo;

use crate::tests::{fetch, fixture, links};

/// Crawls every page of both repositories and fails on any link leaving its `/r/NAME`.
#[tokio::test]
async fn multi_repo_links_stay_inside_their_repo() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = fixture("alpha", dir.path());
    let b = fixture("beta", dir.path());
    let router = crate::multi_router(vec![a, b]).expect("router");
    // The related roots warm in the background: wait for the line before crawling.
    gitterrace_testkit::wait::until("the Related line on alpha's log", || async {
        fetch(&router, "/r/alpha/log/HEAD", "localhost")
            .await
            .1
            .contains("class=\"related\"")
    })
    .await;
    let (s, idx) = fetch(&router, "/", "localhost").await;
    assert_eq!(s, 200);
    assert!(idx.contains("/r/alpha/") && idx.contains("/r/beta/"));
    for name in ["alpha", "beta"] {
        let prefix = format!("/r/{name}/");
        let mut todo = vec![prefix.clone()];
        let mut seen = std::collections::HashSet::new();
        let mut escapes = Vec::new();
        let mut seen_related = std::collections::BTreeSet::new();
        while let Some(u) = todo.pop() {
            if !seen.insert(u.clone()) || seen.len() > 400 {
                continue;
            }
            let (st, body) = fetch(&router, &u, "localhost").await;
            assert!(st < 500, "{u}: {st} {body}");
            if !body.contains("<html") {
                continue;
            }
            let (related, rest) = related_logs(&body, &["alpha", "beta"]);
            if u.starts_with("/r/alpha/log") {
                seen_related.extend(related.iter().cloned());
            }
            for l in links(&rest) {
                // The brand links to the repository index, and stylesheets and scripts
                // are the server's, shared by every repository: both at the root on purpose.
                // The Related line links to related repositories on purpose: its anchors to a
                // served repository's log are taken out of the page by `related_logs`.
                if l == "/" || l.starts_with("/static/") {
                    continue;
                }
                if !l.starts_with(&prefix) {
                    escapes.push(format!("{u} -> {l}"));
                } else if !l.contains("/raw/") {
                    todo.push(l.split('#').next().unwrap_or("").to_owned());
                }
            }
        }
        assert!(seen.len() > 10, "crawl too shallow: {}", seen.len());
        // The exemption stays live: alpha's log shows the Related line, to beta's log only.
        if name == "alpha" {
            let want: std::collections::BTreeSet<String> = ["/r/beta/log".to_owned()].into();
            assert_eq!(seen_related, want, "the Related line on alpha's log");
        }
        assert!(
            escapes.is_empty(),
            "links escape {prefix}:\n{}",
            escapes.join("\n")
        );
    }
    let (s, js) = fetch(&router, "/r/beta/api/files/HEAD", "localhost").await;
    assert_eq!((s, js.contains("src/a.rs")), (200, true));
    assert_eq!(
        fetch(&router, "/r/alpha/api/v1/repo", "localhost").await.0,
        200
    );
    assert_eq!(
        fetch(&router, "/r/alpha/raw/HEAD/src/a.rs", "localhost")
            .await
            .0,
        200
    );
    assert_eq!(fetch(&router, "/static/app.css", "localhost").await.0, 200);
}

/// Takes the Related line's anchors out of `html` when each points at `/r/<name>/log` of a name
/// in `served`, and returns those hrefs and the rest of the page. Any other anchor in the line
/// stays in the page, so the crawl still flags it.
fn related_logs(html: &str, served: &[&str]) -> (Vec<String>, String) {
    const OPEN: &str = "<span class=\"related\">";
    let Some(i) = html.find(OPEN) else {
        return (Vec::new(), html.to_owned());
    };
    let end = html[i..]
        .find("</span>")
        .map_or(html.len(), |e| i + e + "</span>".len());
    let mut span = html[i..end].to_owned();
    let mut hrefs = Vec::new();
    for name in served {
        let href = format!("/r/{name}/log");
        let anchor = format!("<a href=\"{href}\">{name}</a>");
        if span.contains(&anchor) {
            span = span.replace(&anchor, "");
            hrefs.push(href);
        }
    }
    (hrefs, format!("{}{span}{}", &html[..i], &html[end..]))
}

#[tokio::test]
async fn multi_repo_names_resolve_only_through_the_table() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = fixture("alpha", dir.path());
    let b = fixture("beta", dir.path());
    let router = crate::multi_router(vec![a, b]).expect("router");
    for u in [
        "/r/../alpha/",
        "/r/..%2Falpha/",
        "/r/%2e%2e/alpha/",
        "/r/%2E%2E/",
        "/r/alpha%2F..%2Fbeta/",
        "/r/./",
        "/r/gamma/",
        "/r/ALPHA/",
        "/r/alpha%00/",
        "/r//",
        "/r/alpha/../../etc/passwd",
        "/r/alpha/raw/HEAD/../../../etc/passwd",
    ] {
        let (s, body) = fetch(&router, u, "localhost").await;
        assert!(matches!(s, 400 | 404), "{u}: {s} {body}");
        assert!(!body.contains("root:"), "{u}");
    }
    assert_eq!(fetch(&router, "/r/alpha", "localhost").await.0, 308);
}

#[tokio::test]
async fn multi_repo_host_guard_covers_nested_routes() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = fixture("alpha", dir.path());
    let b = fixture("beta", dir.path());
    let router = crate::multi_router(vec![a, b]).expect("router");
    for u in [
        "/",
        "/api/repos",
        "/static/app.css",
        "/r/alpha/",
        "/r/alpha/api/v1/repo",
        "/r/alpha/raw/HEAD/src/a.rs",
        "/r/beta/api/files/HEAD",
        "/r/nope/",
    ] {
        assert_eq!(
            fetch(&router, u, &gitterrace_testkit::on_port("evil.example"))
                .await
                .0,
            403,
            "{u}"
        );
    }
    assert_eq!(
        fetch(
            &router,
            "/r/alpha/api/v1/repo",
            gitterrace_testkit::DEFAULT_ADDR
        )
        .await
        .0,
        200
    );
}

/// Guard (c): two repositories with one folder name. The first to arrive keeps the plain
/// name, the newcomer gets `-<hash6>`, and both names survive a restart in the other order:
/// they come from the registry, never from arrival order.
#[test]
fn a_clashing_folder_name_is_first_come_and_frozen() {
    let dir = tempfile::tempdir().expect("tmp");
    let state = dir.path().join("state");
    let z = fixture("app", &dir.path().join("z"));
    let a = fixture("app", &dir.path().join("a"));
    let env = || crate::app::Env::new(gitterrace_config::Config::default());
    let names = |paths: Vec<PathBuf>| {
        let t = crate::multi::Table::new(paths.clone(), env(), Some(state.clone())).expect("table");
        paths
            .iter()
            .map(|p| {
                let root = Repo::discover(p).expect("repo").root();
                t.name_of(&root).expect("served")
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(names(vec![z.clone()]), ["app"], "alone it is plain");
    let both = names(vec![z.clone(), a.clone()]);
    assert_eq!(both[0], "app", "the first come keeps it");
    assert!(
        both[1].starts_with("app-") && both[1].len() == 10,
        "{both:?}"
    );
    let reversed = names(vec![a, z]);
    assert_eq!(reversed, [both[1].clone(), both[0].clone()], "frozen");
}

/// Without a registry the same rule runs against the table: a clash still gets a stable
/// path-derived suffix, never a counter that depends on order.
#[test]
fn without_a_registry_a_clash_is_named_by_its_path() {
    let dir = tempfile::tempdir().expect("tmp");
    let x = fixture("same", &dir.path().join("x"));
    let y = fixture("same", &dir.path().join("y"));
    let env = || crate::app::Env::new(gitterrace_config::Config::default());
    let t = crate::multi::Table::new(vec![x.clone(), y.clone()], env(), None).expect("t");
    let y_root = Repo::discover(&y).expect("repo").root();
    let y_name = t.name_of(&y_root).expect("served");
    assert!(t.get("same").is_some() && y_name.starts_with("same-"));
    let t = crate::multi::Table::new(vec![y, x], env(), None).expect("t");
    assert_eq!(
        t.name_of(&y_root).as_deref(),
        Some("same"),
        "no registry, no memory"
    );
}

#[tokio::test]
async fn a_single_repository_lives_under_its_name() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = fixture("solo", dir.path());
    let router = crate::multi_router(vec![a]).expect("router");
    let (s, _) = fetch(&router, "/", "localhost").await;
    assert_eq!(s, 302, "/ forwards to the one repository");
    let (_, body) = fetch(&router, "/r/solo/", "localhost").await;
    assert!(body.contains("href=\"/r/solo/log/HEAD\""));
    assert!(body.contains("<meta name=\"gt-base\" content=\"/r/solo\">"));
    assert!(
        body.contains("id=\"reposwitch\" hidden"),
        "one repository, no switcher"
    );
    assert_eq!(
        fetch(&router, "/log/HEAD", "localhost").await.0,
        404,
        "no unprefixed pages"
    );
}

/// The repositories home leads with today: a repository with a commit from this hour says how
/// many landed and the newest subject, a quiet one says so, and uncommitted work shows its dot.
#[tokio::test]
async fn the_repositories_home_says_what_changed_today() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = fixture("alpha", dir.path());
    // The fixture commits now; beta is rebuilt with one commit years ago, so it is quiet.
    let b = dir.path().join("beta");
    std::fs::create_dir_all(b.join("src")).expect("mkdir");
    gitterrace_testkit::git(&b, &["init", "-q", "-b", "main"]);
    std::fs::write(b.join("src/a.rs"), "fn main() {}\n").expect("write");
    gitterrace_testkit::git(&b, &["add", "-A"]);
    gitterrace_testkit::git_at(&b, "2020-01-01T12:00:00Z", &["commit", "-qm", "feat: old"]);
    std::fs::write(b.join("src/a.rs"), "// edited, not committed\n").expect("write");
    let router = crate::multi_router(vec![a, b]).expect("router");
    let (s, body) = fetch(&router, "/", "localhost").await;
    assert_eq!(s, 200);
    assert!(
        body.contains("2 commits today, last: fix: second"),
        "{body}"
    );
    assert!(body.contains("Quiet today. Last commit"), "{body}");
    assert!(
        body.contains("<b>2</b> repositories, <b>2</b> commits today"),
        "{body}"
    );
    // The chips fill in after the rows: a cold page renders every one hidden.
    assert!(
        !body.contains("working tree\"><"),
        "no chip before a scan: {body}"
    );
    let (_, dirty) = fetch(&router, "/_dirty", "localhost").await;
    assert_eq!(
        dirty, r#"{"beta":true}"#,
        "beta has uncommitted work, alpha is clean"
    );
}

/// `/?q=` narrows the list on the server, so the filter works without a script and a link
/// carries it: the other repository stays out of the page's rows, the count says so, and a
/// filter nothing matches says that quietly.
#[tokio::test]
async fn the_repositories_home_filters_by_name_on_the_server() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = fixture("alpha", dir.path());
    let b = fixture("beta", dir.path());
    let router = crate::multi_router(vec![a, b]).expect("router");
    let shown = |body: &str, name: &str| body.contains(&format!("data-name=\"{name}\">"));
    let (_, all) = fetch(&router, "/", "localhost").await;
    assert!(shown(&all, "alpha") && shown(&all, "beta"), "{all}");
    assert!(all.contains(">2 of 2<"), "{all}");
    let (s, one) = fetch(&router, "/?q=%20ALP", "localhost").await;
    assert_eq!(s, 200);
    assert!(shown(&one, "alpha") && !shown(&one, "beta"), "{one}");
    assert!(
        one.contains(">1 of 2<") && one.contains("value=\" ALP\""),
        "{one}"
    );
    let (_, zero) = fetch(&router, "/?q=zzz", "localhost").await;
    assert!(zero.contains(">0 of 2<"), "{zero}");
    assert!(!zero.contains("repos-none\" hidden"), "{zero}");
}
