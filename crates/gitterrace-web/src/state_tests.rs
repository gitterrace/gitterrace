//! Tests for revisions, git state, read-only routes, public mode and binding.

#[cfg(unix)]
use axum::{body::Body, http::Request};
use gitterrace_testkit::{Fixture, git};
#[cfg(unix)]
use tower::ServiceExt;

#[cfg(unix)]
use crate::actions::actions_tests;
#[cfg(unix)]
use crate::tests::fetch;
use crate::tests::{fixture, get, links, ready_app};

fn rev_fixture(root: &std::path::Path) -> std::path::PathBuf {
    let f = Fixture::standard(&root.join("solo"))
        .branch("side")
        .write("side.txt", "s\n")
        .commit("feat: only on side")
        .checkout("main")
        .tag("v1", "Release one\n\nnotes here")
        .worktree(&root.join("wt"), "side")
        .write("README.md", "dirty\n")
        .stash("wip stash");
    f.path().to_owned()
}

/// A cache a newer build wrote is explained on the insights pages, served as 200, and left
/// byte-identical: an older server running next to a newer one must not clobber it.
#[tokio::test]
async fn a_newer_cache_is_explained_not_clobbered() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = rev_fixture(dir.path());
    let built = ready_app(&p);
    let cache = built.index.dir().to_owned();
    drop(built);
    std::fs::write(cache.join("VERSION"), "v99\n").expect("stamp");
    let snapshot = |d: &std::path::Path| {
        let mut v: Vec<_> = std::fs::read_dir(d)
            .expect("ls")
            .map(|e| {
                let e = e.expect("entry");
                (e.file_name(), std::fs::read(e.path()).expect("read"))
            })
            .collect();
        v.sort();
        v
    };
    let before = snapshot(&cache);
    let app = crate::tests::new_app(&p);
    // The server marks the app ready once its update attempt ends, failed or not.
    app.mark_ready();
    for uri in ["/stats", "/stats/languages", "/stats/hotspots"] {
        let (s, body) = get(&app, uri).await;
        assert_eq!(s, 200, "{uri}");
        assert!(body.contains("format 99"), "{uri}: {body}");
    }
    assert!(snapshot(&cache) == before, "cache files changed");
}

#[tokio::test]
async fn git_state_pages_and_banner() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = rev_fixture(dir.path());
    let app = ready_app(&p);
    let (s, body) = get(&app, "/refs").await;
    assert_eq!(s, 200);
    assert!(body.contains(">side</a>") && body.contains("default"), "{body}");
    assert!(body.contains("/compare?base=main&#38;head=side"), "ahead links to compare");
    assert!(body.contains("Release one") && body.contains("/tag/v1"));
    let (s, body) = get(&app, "/tag/v1").await;
    assert!(s == 200 && body.contains("notes here") && body.contains("Tagged by"));
    assert_eq!(get(&app, "/tag/nope").await.0, 404);
    let (_, body) = get(&app, "/refs").await;
    assert!(body.contains("/wt</code>") && body.contains("/log/side"), "{body}");
    assert!(body.contains("stash@{0}") && body.contains("wip stash"));
    assert!(body.contains("id=\"headstate\" hidden>"), "the banner is hidden on a branch");
    let head = app.repo.resolve("HEAD").expect("head");
    git(&p, &["checkout", "-q", "--detach"]);
    let (_, body) = get(&app, "/").await;
    let short = gitterrace_git::short_id(&head);
    assert!(
        body.contains(&format!(
            "id=\"headstate\"><div class=\"wrap\">HEAD is detached at <a \
            class=\"sha\" href=\"/commit/{head}\" data-tip=\"{head}\">{short}</a>"
        )),
        "{body}"
    );
}

/// The selected ref survives every link to a page computed from it, and those pages read
/// the selected ref, not HEAD.
#[tokio::test]
async fn rev_is_preserved_by_every_link() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = rev_fixture(dir.path());
    let app = ready_app(&p);
    // A ref a few commits off HEAD is served from the segments at once, no indexing page.
    let (s, body) = get(&app, "/stats?rev=side").await;
    assert!(s == 200 && !body.contains("data-progress"), "side shows the indexing page: {body}");
    let (_, main) = get(&app, "/stats").await;
    let (_, side) = get(&app, "/stats?rev=side").await;
    assert!(side.contains("only on side") && !main.contains("only on side"));
    let (_, side) = get(&app, "/?rev=side").await;
    assert!(side.contains("side.txt"), "overview lists the selected ref's files");
    let mut todo = vec!["/?rev=side".to_owned(), "/stats?rev=side".to_owned()];
    let mut seen = std::collections::HashSet::new();
    let mut lost = Vec::new();
    while let Some(u) = todo.pop() {
        if !seen.insert(u.clone()) || seen.len() > 300 {
            continue;
        }
        let (st, body) = get(&app, &u).await;
        assert!(st < 500, "{u}: {st}");
        if !body.contains("<html") {
            continue;
        }
        // The brand goes to the repository index on purpose, whatever ref is selected.
        let body = body.replacen("class=\"brand\" href=\"/\"", "class=\"brand\"", 1);
        for l in links(&body) {
            let path = l.split(['?', '#']).next().unwrap_or_default().to_owned();
            let follows = path == "/"
                || path == "/stats"
                || path.starts_with("/stats/")
                || path.starts_with("/history/");
            if follows && !l.contains("rev=side") {
                lost.push(format!("{u} -> {l}"));
            } else if follows {
                todo.push(l.split('#').next().unwrap_or_default().to_owned());
            }
        }
    }
    assert!(seen.len() > 10, "crawl too shallow: {}", seen.len());
    assert!(lost.is_empty(), "links drop ?rev=:\n{}", lost.join("\n"));
}

/// Methods a route may answer besides GET and HEAD, as (path, method). gitterrace is
/// read-only, so a write route is an owner decision recorded here, never an accident.
/// The one entry is the action runner, mounted only when the user configured actions.
#[cfg(unix)]
const WRITE_ALLOWLIST: [(&str, &str); 1] = [(crate::actions::RUN_PATH, "POST")];

/// Every page a crawl reaches, the API and the assets refuse any method but GET and HEAD,
/// with actions off and on. With them off the allowlisted route does not exist at all.
#[cfg(unix)]
#[tokio::test]
async fn every_route_is_read_only() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = fixture("alpha", dir.path());
    let b = fixture("beta", dir.path());
    let cfg = actions_tests::config_dir(actions_tests::ECHO);
    let off = crate::multi_router(vec![a.clone(), b.clone()]).expect("router");
    let on = crate::build_multi(
        vec![a, b],
        None,
        Some(&cfg.path().join("actions.toml")),
        &crate::app::Env::new(gitterrace_config::Config::default()),
    )
    .expect("router")
    .0;
    for (router, actions) in [(off.clone(), false), (on, true)] {
        crawl_read_only(&router, actions).await;
    }
    let res = off
        .oneshot(
            Request::post(crate::actions::RUN_PATH)
                .header("host", "localhost")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("res");
    assert_eq!(res.status(), 404, "run route exists with actions off");
}

#[cfg(unix)]
async fn crawl_read_only(router: &axum::Router, actions: bool) {
    let router = router.clone();
    let mut todo = vec!["/".to_owned(), "/r/alpha/".to_owned(), "/api/repos".to_owned()];
    todo.extend(
        ["/api/v1/repo", "/api/v1/refs", "/api/progress", "/api/refs", "/api/files/HEAD"]
            .map(|p| format!("/r/alpha{p}")),
    );
    let mut seen = std::collections::HashSet::new();
    let mut writable = Vec::new();
    // The run route renders no link, so the crawl is seeded with it; GET on it is 405.
    todo.push(crate::actions::RUN_PATH.to_owned());
    while let Some(u) = todo.pop() {
        if !seen.insert(u.clone()) || seen.len() > 300 {
            continue;
        }
        let (get_status, body) = fetch(&router, &u, "localhost").await;
        for l in links(&body) {
            todo.push(l.split('#').next().unwrap_or_default().to_owned());
        }
        for m in ["POST", "PUT", "PATCH", "DELETE"] {
            let res = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(m)
                        .uri(&u)
                        .header("host", "localhost")
                        .body(Body::empty())
                        .expect("req"),
                )
                .await
                .expect("res");
            let path = u.split('?').next().unwrap_or_default();
            let allowed = actions && WRITE_ALLOWLIST.iter().any(|(p, am)| *p == path && *am == m);
            // 405 is a route that exists for GET only. A URL that GET cannot find either
            // (a `data-last` prefix) may answer 404; any other status means a handler took
            // the write.
            let code = res.status().as_u16();
            let refused = code == 405 || (code == 404 && get_status == 404);
            if !refused && !allowed {
                writable.push(format!("{m} {u}: {}", res.status()));
            }
        }
    }
    assert!(seen.len() > 20, "crawl too shallow: {}", seen.len());
    assert!(writable.is_empty(), "routes accept writes:\n{}", writable.join("\n"));
}

#[cfg(unix)]
fn public_cfg() -> gitterrace_config::Config {
    let mut c = gitterrace_config::Config::default();
    c.server.is_public = true;
    c.server.hosts = vec!["git.example.com".to_owned()];
    c
}

/// A public server, crawled from `/` through every link it renders and seeded with every
/// local route: each answer is a classified public route, none names the host's folders or
/// local state, local routes are 404, writes are 405 before routing.
#[cfg(unix)]
#[tokio::test]
async fn public_server_shows_committed_history_only() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture("alpha", dir.path());
    std::fs::write(p.join("wip.txt"), "uncommitted-secret\n").expect("write");
    git(&p, &["stash", "-q", "-u"]);
    std::fs::write(p.join("dirty.txt"), "dirty-secret\n").expect("write");
    let cfg = actions_tests::config_dir(actions_tests::ECHO);
    let env = crate::app::Env::new(public_cfg());
    let router =
        crate::build(&p, None, Some(&cfg.path().join("actions.toml")), &env).expect("router").0;
    let root = std::fs::canonicalize(&p).expect("canon");
    let leaks = [
        root.to_string_lossy().into_owned(),
        dir.path().to_string_lossy().into_owned(),
        "uncommitted-secret".to_owned(),
        "dirty-secret".to_owned(),
    ];
    let mut todo = vec![
        "/".to_owned(),
        "/r/alpha/".to_owned(),
        "/r/alpha/api/v1/repo".to_owned(),
        "/r/alpha/api/v1/refs".to_owned(),
    ];
    let mut seen = std::collections::HashSet::new();
    let mut bad = Vec::new();
    while let Some(u) = todo.pop() {
        if !seen.insert(u.clone()) || seen.len() > 300 {
            continue;
        }
        let (s, body) = fetch(&router, &u, "git.example.com").await;
        if s == 200 {
            for l in links(&body) {
                todo.push(l.split('#').next().unwrap_or_default().to_owned());
            }
        }
        if let Some(l) = leaks.iter().find(|l| body.contains(l.as_str())) {
            bad.push(format!("{u} shows {l}"));
        }
        for l in ["/worktree", "/stashes", "/events", "/_send", "/_actions"] {
            if body.contains(&format!("href=\"{l}"))
                || body.contains(&format!("href=\"/r/alpha{l}"))
            {
                bad.push(format!("{u} links {l}"));
            }
        }
    }
    assert!(seen.len() > 20, "crawl too shallow: {}", seen.len());
    for u in [
        "/r/alpha/worktree",
        "/r/alpha/worktrees",
        "/r/alpha/stashes",
        "/r/alpha/events",
        "/_actions/status",
        "/r/alpha/_send/targets",
        "/r/alpha/commit/stash",
        "/r/alpha/log/refs/stash",
        "/r/alpha/tree/stash",
        "/r/alpha/api/v1/log?rev=stash",
        "/r/alpha/api/v1/blob?rev=stash&path=wip.txt",
    ] {
        let (s, body) = fetch(&router, u, "git.example.com").await;
        if s == 200 || body.contains("uncommitted-secret") {
            bad.push(format!("{u}: {s}"));
        }
    }
    for (m, u) in [("POST", crate::actions::RUN_PATH), ("POST", "/_repos"), ("PUT", "/")] {
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(m)
                    .uri(u)
                    .header("host", "git.example.com")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        if res.status() != 405 {
            bad.push(format!("{m} {u}: {}", res.status()));
        }
    }
    assert!(bad.is_empty(), "public server leaks:\n{}", bad.join("\n"));
    // The proxy name works, a cross-site link into it works, another name does not.
    let res = router
        .clone()
        .oneshot(
            Request::get("/r/alpha/")
                .header("host", "GIT.example.com:443")
                .header("sec-fetch-site", "cross-site")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("res");
    assert_eq!(res.status(), 200);
    assert_eq!(fetch(&router, "/", "evil.example.com").await.0, 403);
}

/// A taken port names the address and the way out, not a bare os error.
#[tokio::test]
async fn a_busy_port_names_the_address_and_a_hint() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture("alpha", dir.path());
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = taken.local_addr().expect("addr");
    let e = crate::serve_in(vec![p], addr, None, gitterrace_config::Config::default(), None, None)
        .await
        .expect_err("bound a taken port");
    let msg = e.to_string();
    assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse);
    assert!(msg.contains(&addr.to_string()), "{msg}");
    assert!(msg.contains("--addr"), "{msg}");
}

/// `serve` refuses a reachable address unless the server is public, before binding.
#[tokio::test]
async fn a_reachable_bind_needs_public() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture("alpha", dir.path());
    let addr: std::net::SocketAddr = "0.0.0.0:0".parse().expect("addr");
    let e = crate::serve_in(vec![p], addr, None, gitterrace_config::Config::default(), None, None)
        .await
        .expect_err("served without public");
    assert!(e.to_string().contains("--public"), "{e}");
}

/// A missing file or revision answers 404 inside the app shell, with the header nav and the
/// next steps, and a script-read fragment still gets the bare answer.
#[tokio::test]
async fn not_found_keeps_the_shell_and_offers_next_steps() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = rev_fixture(dir.path());
    let app = ready_app(&p);
    let (s, body) = get(&app, "/blob/v1/src/nope.c").await;
    assert_eq!(s, 404);
    assert!(body.contains("class=\"tabnav\"") && body.contains("id=\"q\""), "{body}");
    assert!(body.contains("No file at src/nope.c on v1"), "{body}");
    assert!(body.contains("Search for nope.c") && body.contains("/history/src/nope.c"));
    assert!(body.contains("/tree/v1\">Back to the tree of v1"), "{body}");
    let (s, body) = get(&app, "/log/nope").await;
    assert!(s == 404 && body.contains("No revision nope") && body.contains("class=\"tabnav\""));
    let (s, body) = get(&app, "/nothing/here").await;
    assert!(
        s == 404 && body.contains("Nothing at this address") && body.contains("class=\"tabnav\"")
    );
    let (s, body) = get(&app, "/diff/nope/0").await;
    assert!(s == 404 && !body.contains("class=\"tabnav\""), "{body}");
}

/// `/refs` on a repository with thousands of refs reads commits and walks divergences for the
/// rows it shows, never for every ref: the page's cost is bounded by `REFS_SHOWN`, and the
/// counts say how many exist. `?all=1` still lists every one.
#[tokio::test]
async fn refs_reads_only_the_rows_it_shows() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut f = Fixture::init(&dir.path().join("many"));
    for i in 0..20 {
        f = f.write("a.txt", &format!("{i}\n")).commit(&format!("feat: c{i}"));
    }
    let p = f.path().to_owned();
    let tips: Vec<String> =
        (0..20).map(|i| git(&p, &["rev-parse", &format!("HEAD~{i}")]).trim().to_owned()).collect();
    let lines: Vec<String> = (0..1000)
        .map(|i| {
            let tip = &tips[i % tips.len()];
            format!("create refs/tags/t{i:04} {tip}\ncreate refs/heads/b{i:04} {tip}\n")
        })
        .collect();
    gitterrace_testkit::git_stdin(&p, &["update-ref", "--stdin"], lines.concat().as_bytes());
    let app = ready_app(&p);
    let before = app.repo.row_reads();
    let (s, body) = get(&app, "/refs").await;
    assert_eq!(s, 200);
    let reads = app.repo.row_reads() - before;
    let shown = crate::routes::REFS_SHOWN as u64;
    // Per shown row: one commit read, and for a branch one divergence walk.
    assert!(reads <= 3 * shown, "{reads} reads for {shown} rows shown of 2001 refs");
    assert!(body.contains("1,000 tags, 100 shown."), "honest tag count");
    assert!(body.contains("1,001 branches, 100 shown."), "honest branch count");
    assert!(body.contains("Show all 1,000 tags"));
    let (s, all) = get(&app, "/refs?all=1").await;
    assert_eq!(s, 200);
    assert!(all.contains("/tag/t0999") && all.contains("/log/b0999"), "all rows on ?all=1");
    assert!(!all.contains("shown."), "no cut on ?all=1");
}
