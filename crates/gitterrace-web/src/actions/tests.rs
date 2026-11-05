use gitterrace_testkit::git;
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::actions::RUN_PATH;
use crate::actions::actions_tests::{ECHO, config_dir, write_config};

/// Repo content aimed at the page and the shell: every field the menu touches.
const XSS: &str = "<script>alert(1)</script>\"><img src=x onerror=alert(2)>";

/// A repo whose subject, author, branch, path and content carry HTML and shell payloads,
/// and which ships its own actions.toml (which must never load).
fn hostile(root: &Path) -> (PathBuf, String) {
    let p = root.join("evil");
    std::fs::create_dir_all(p.join("src")).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    git(&p, &["config", "user.name", &format!("Eve {XSS}")]);
    git(&p, &["config", "user.email", "e@x.y"]);
    std::fs::write(
        p.join("src/a.rs"),
        format!("fn a() {{}}\n// {XSS}\n// $(touch pwned)\n"),
    )
    .expect("w");
    std::fs::write(p.join(format!("src/{}.txt", "q\"><img onerror=x>")), "x\n").expect("w");
    std::fs::write(p.join("actions.toml"), "version = 1\n").expect("w");
    std::fs::create_dir_all(p.join(".gitterrace")).expect("dir");
    std::fs::write(p.join(".gitterrace/actions.toml"), "version = 1\n").expect("w");
    git(&p, &["add", "."]);
    git(
        &p,
        &["commit", "-q", "-m", &format!("fix: $(touch pwned) {XSS}")],
    );
    git(&p, &["branch", "x\"><svg-onload=alert(3)>"]);
    let sha = git(&p, &["rev-parse", "HEAD"]);
    (p, sha)
}

fn router(repo: &Path, cfg: Option<&Path>, bound: Option<std::net::IpAddr>) -> axum::Router {
    crate::build_multi(
        vec![repo.to_path_buf()],
        bound,
        cfg.map(|d| d.join("actions.toml")).as_deref(),
        &crate::app::Env::new(gitterrace_config::Config::default()),
    )
    .expect("router")
    .0
}

/// GETs `uri`; a repository page path is taken under the one repository's `/r/evil`, the
/// server's own routes (`/go`, `/_send`, `/_actions`) as they are.
async fn page(r: &axum::Router, uri: &str) -> (u16, String) {
    let server = ["/go/", "/_"].iter().any(|p| uri.starts_with(p));
    let uri = if server {
        uri.to_owned()
    } else {
        format!("/r/evil{uri}")
    };
    let res = r
        .clone()
        .oneshot(
            Request::get(uri)
                .header("host", gitterrace_testkit::on_port("localhost"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("res");
    let s = res.status().as_u16();
    let b = res.into_body().collect().await.expect("body").to_bytes();
    (s, String::from_utf8_lossy(&b).into_owned())
}

fn token(html: &str) -> String {
    let i = html
        .find("name=\"gitterrace-actions\" content=\"")
        .expect("token meta");
    let rest = &html[i + 35..];
    rest[..rest.find('"').expect("end")].to_owned()
}

struct Req<'a> {
    token: Option<&'a str>,
    site: Option<&'a str>,
    host: String,
    origin: Option<String>,
    ctype: &'a str,
    body: String,
}

impl<'a> Req<'a> {
    fn ok(token: &'a str, body: String) -> Self {
        Self {
            token: Some(token),
            site: Some("same-origin"),
            host: gitterrace_testkit::on_port("localhost"),
            origin: Some(format!(
                "http://{}",
                gitterrace_testkit::on_port("localhost")
            )),
            ctype: "application/json",
            body,
        }
    }
}

async fn send(r: &axum::Router, q: Req<'_>) -> (u16, serde_json::Value) {
    let mut b = Request::post(RUN_PATH)
        .header("host", q.host)
        .header("content-type", q.ctype);
    if let Some(t) = q.token {
        b = b.header("x-gitterrace-token", t);
    }
    if let Some(s) = q.site {
        b = b.header("sec-fetch-site", s);
    }
    if let Some(o) = q.origin {
        b = b.header("origin", o);
    }
    let res = r
        .clone()
        .oneshot(b.body(Body::from(q.body)).expect("req"))
        .await
        .expect("res");
    let s = res.status().as_u16();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    (
        s,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn body(action: &str, kind: &str, extra: &str) -> String {
    format!(
        r#"{{"action":"{action}","kind":"{kind}","locator":{{"repo":"evil","rev":"main"{extra}}}}}"#
    )
}

/// The design's table: every way a request can be wrong, then the one way it is right.
#[tokio::test(flavor = "multi_thread")]
async fn run_route_security_table() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (repo, sha) = hostile(tmp.path());
    let cfg = config_dir(ECHO);

    // 1. not configured: no route, no token in pages.
    let none = tempfile::tempdir().expect("tmp");
    let off = router(&repo, Some(none.path()), None);
    let (_, html) = page(&off, "/").await;
    assert!(
        !html.contains("gitterrace-actions"),
        "token without actions"
    );
    let (s, _) = send(&off, Req::ok("x", body("echo", "page", ""))).await;
    assert_eq!(s, 404, "not configured");
    // 2. --no-actions: the CLI passes no file.
    let (s, _) = send(
        &router(&repo, None, None),
        Req::ok("x", body("echo", "page", "")),
    )
    .await;
    assert_eq!(s, 404, "--no-actions");
    // 3. bound to the LAN or to every interface.
    for ip in ["0.0.0.0", "192.168.1.5"] {
        let lan = router(&repo, Some(cfg.path()), Some(ip.parse().expect("ip")));
        let mut q = Req::ok("x", body("echo", "page", ""));
        let host = if ip == "0.0.0.0" { "127.0.0.1" } else { ip };
        q.host = gitterrace_testkit::on_port(host);
        q.origin = Some(format!("http://{}", q.host));
        assert_eq!(send(&lan, q).await.0, 404, "bound to {ip}");
    }

    let on = router(&repo, Some(cfg.path()), None);
    let (_, html) = page(&on, "/").await;
    let tok = token(&html);
    assert_eq!(tok.len(), 43);
    let wrong = {
        let mut b = tok.clone().into_bytes();
        b[5] = if b[5] == b'A' { b'B' } else { b'A' };
        String::from_utf8(b).expect("ascii")
    };
    let ok = || Req::ok(&tok, body("echo", "page", ""));
    // 4-5. missing and one-byte-off tokens.
    let mut q = ok();
    q.token = None;
    assert_eq!(send(&on, q).await.0, 403, "missing token");
    let mut q = ok();
    q.token = Some(&wrong);
    assert_eq!(send(&on, q).await.0, 403, "wrong token");
    // 6-8. fetch metadata: cross-site, same-site (another port), none and absent.
    for site in [Some("cross-site"), Some("same-site"), Some("none"), None] {
        let mut q = ok();
        q.site = site;
        assert_eq!(send(&on, q).await.0, 403, "sec-fetch-site {site:?}");
    }
    // 9. DNS rebinding.
    let mut q = ok();
    q.host = "evil.example".to_owned();
    q.origin = Some("http://evil.example".to_owned());
    assert_eq!(send(&on, q).await.0, 403, "foreign host");
    // 10. another localhost origin, and no origin at all.
    for origin in [Some("http://localhost:5173"), Some("null"), None] {
        let mut q = ok();
        q.origin = origin.map(str::to_owned);
        assert_eq!(send(&on, q).await.0, 403, "origin {origin:?}");
    }
    // 11. not JSON.
    for ctype in ["text/plain", "application/x-www-form-urlencoded"] {
        let mut q = ok();
        q.ctype = ctype;
        assert_eq!(send(&on, q).await.0, 415, "{ctype}");
    }
    // 12. unknown action, kind not allowed, unknown field, a command field.
    for b in [
            body("nope", "page", ""),
            body("echo", "hunk", r#","path":"src/a.rs""#),
            body("echo", "page", r#","cwd":"/""#),
            r#"{"action":"echo","kind":"page","command":["sh"],"locator":{"repo":"evil","rev":"main"}}"#
                .to_owned(),
            body("echo", "page", "").replace("evil", "other"),
        ] {
            assert_eq!(send(&on, Req::ok(&tok, b.clone())).await.0, 400, "{b}");
        }
    // 13. path traversal.
    let (s, _) = send(
        &on,
        Req::ok(&tok, body("echo", "file", r#","path":"../../etc/passwd""#)),
    )
    .await;
    assert_eq!(s, 400, "traversal");

    // 14. valid: exact JSON on stdin, only allowed env. 15. $(touch pwned) not run.
    let (s, res) = send(&on, Req::ok(&tok, body("echo", "commit", ""))).await;
    assert_eq!(s, 200, "{res}");
    assert_eq!(res["ok"], true);
    assert!(
        res["first_line"]
            .as_str()
            .expect("line")
            .contains("$(touch pwned)")
    );
    assert!(!repo.join("pwned").exists() && !cfg.path().join("pwned").exists());
    let stdin: serde_json::Value =
        serde_json::from_slice(&std::fs::read(cfg.path().join("stdin.json")).expect("stdin"))
            .expect("json");
    assert_eq!(stdin["v"], 1);
    assert_eq!(stdin["kind"], "commit");
    assert_eq!(stdin["sha"], sha);
    assert_eq!(stdin["repo"]["name"], "evil");
    assert!(
        stdin["permalink"]
            .as_str()
            .expect("link")
            .starts_with("gitterrace://")
    );
    let env = std::fs::read_to_string(cfg.path().join("env.txt")).expect("env");
    for line in env.lines() {
        let name = line.split('=').next().unwrap_or_default();
        assert!(
            [
                "PATH", "HOME", "USER", "LANG", "TMPDIR", "PWD", "SHLVL", "OLDPWD", "_"
            ]
            .contains(&name)
                || name.starts_with("LC_")
                || name.starts_with("GITTERRACE_"),
            "leaked {name}"
        );
    }
    let (s, res) = send(
        &on,
        Req::ok(
            &tok,
            body(
                "echo",
                "line",
                r#","path":"src/a.rs","lines":{"start":2,"end":3}"#,
            ),
        ),
    )
    .await;
    assert_eq!(
        (s, &res["ok"]),
        (200, &serde_json::Value::Bool(true)),
        "{res}"
    );

    // 16. timeout: killed, its grandchild too.
    let (s, res) = send(&on, Req::ok(&tok, body("slow", "page", ""))).await;
    assert_eq!(s, 200);
    assert_eq!(res["ok"], false);
    assert!(
        res["first_line"]
            .as_str()
            .expect("line")
            .contains("timed out")
    );
    let pid = std::fs::read_to_string(cfg.path().join("grandchild.pid")).expect("pid");
    // The group was signalled; the kernel reaps the grandchild soon after, well before its
    // own 30 s sleep would end it.
    let alive = || {
        Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .expect("kill")
            .success()
    };
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while alive() && std::time::Instant::now() < end {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!alive(), "grandchild survived");

    // 17. the 5th concurrent run is refused.
    let mut runs = Vec::new();
    for id in ["nap1", "nap2", "nap3", "nap4"] {
        let (r, t) = (on.clone(), tok.clone());
        runs.push(tokio::spawn(async move {
            send(&r, Req::ok(&t, body(id, "page", ""))).await.0
        }));
    }
    // Each nap marks itself started and runs until released, so all four hold a slot now.
    let up = |id: &str| cfg.path().join(format!("{id}.up")).exists();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !["nap1", "nap2", "nap3", "nap4"].iter().all(|id| up(id))
        && std::time::Instant::now() < end
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        ["nap1", "nap2", "nap3", "nap4"].iter().all(|id| up(id)),
        "four runs started"
    );
    let (s, res) = send(&on, Req::ok(&tok, body("echo", "page", ""))).await;
    std::fs::write(cfg.path().join("release"), "").expect("release");
    assert_eq!(s, 429, "5th concurrent: {res}");
    for r in runs {
        assert_eq!(r.await.expect("join"), 200);
    }

    // 18. the repo's own actions.toml files never load: naming one is refused.
    for local in [repo.clone(), repo.join(".gitterrace")] {
        let r = crate::build_multi(
            vec![repo.clone()],
            None,
            Some(&local.join("actions.toml")),
            &crate::app::Env::new(gitterrace_config::Config::default()),
        )
        .expect("router")
        .0;
        assert_eq!(
            send(&r, Req::ok("x", body("echo", "page", ""))).await.0,
            404,
            "{}",
            local.display()
        );
    }
    // 19. group-writable config: refused, no route.
    let gw = tempfile::tempdir().expect("tmp");
    write_config(gw.path(), ECHO, 0o620);
    let r = router(&repo, Some(gw.path()), None);
    assert_eq!(
        send(&r, Req::ok("x", body("echo", "page", ""))).await.0,
        404,
        "group writable"
    );
}

/// Repo content shows up escaped on every page that carries the action menu.
#[tokio::test(flavor = "multi_thread")]
async fn hostile_repo_content_is_escaped_where_the_menu_lives() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (repo, sha) = hostile(tmp.path());
    let cfg = config_dir(ECHO);
    let r = router(&repo, Some(cfg.path()), None);
    let branch = "x%22%3E%3Csvg-onload%3Dalert%283%29%3E";
    for uri in [
        "/".to_owned(),
        format!("/commit/{sha}"),
        "/blob/main/src/a.rs".to_owned(),
        "/blame/main/src/a.rs".to_owned(),
        "/tree/main/src".to_owned(),
        format!("/log/{branch}"),
        "/refs".to_owned(),
        "/search?q=script".to_owned(),
        "/stats/hotspots".to_owned(),
        format!("/tree/{branch}"),
    ] {
        let (s, html) = page(&r, &uri).await;
        assert!(s == 200 || s == 404, "{uri}: {s}");
        for raw in [
            "<script>alert(1)",
            "<img src=x onerror",
            "\"><img onerror",
            "\"><svg-onload",
        ] {
            assert!(!html.contains(raw), "{uri} renders {raw:?} unescaped");
        }
        if s == 200 {
            assert!(
                html.contains("name=\"gitterrace-actions\""),
                "{uri} lacks the menu token"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn go_redirects_to_the_repo_holding_the_commit() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (repo, sha) = hostile(tmp.path());
    let id = gitterrace_git::Repo::discover(&repo)
        .expect("repo")
        .id()
        .expect("id");
    let r = router(&repo, None, None);
    let loc = |res: &axum::response::Response| {
        res.headers()
            .get("location")
            .map(|v| v.to_str().expect("ascii").to_owned())
    };
    for (uri, want) in [
        (
            format!("/go/{id}/{sha}"),
            Some(format!("/r/evil/commit/{sha}")),
        ),
        (
            format!("/go/{id}/{sha}/src/a.rs"),
            Some(format!("/r/evil/blob/{sha}/src/a.rs")),
        ),
        (
            format!("/go/{id}/{sha}/src"),
            Some(format!("/r/evil/tree/{sha}/src")),
        ),
        (format!("/go/{id}/{sha}/../../etc/passwd"), None),
        (format!("/go/000000000000/{sha}"), None),
        (format!("/go/{id}/{}", &sha[..12]), None),
    ] {
        let res = r
            .clone()
            .oneshot(
                Request::get(&uri)
                    .header("host", "localhost")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        match want {
            Some(w) => assert_eq!((res.status().as_u16(), loc(&res)), (302, Some(w)), "{uri}"),
            None => assert_eq!(res.status(), 404, "{uri}"),
        }
    }
    let (s, html) = page(&r, "/go/%3Cimg%20src%3Dx%3E/abc").await;
    assert_eq!(s, 404);
    assert!(html.contains("&lt;img src=x&gt;") && !html.contains("<img src=x>"));
    let (_, html) = page(&r, "/").await;
    assert!(
        html.contains(&format!("data-id=\"{id}\"")),
        "permalink id not in the page"
    );
}

/// Targets come back as parts with `var` already resolved; no file or a file with no
/// `[[send]]` is `[]`, and sending needs no action.
#[tokio::test(flavor = "multi_thread")]
async fn send_targets_are_template_parts() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (repo, _) = hostile(tmp.path());
    let (status, body) = page(&router(&repo, None, None), "/_send/targets").await;
    assert_eq!((status, body.as_str()), (200, "[]"));
    let cfg = config_dir(
        "version = 1\n[[send]]\nid = \"gh\"\nlabel = \"New issue\"\nurl = \"https://github.com/{var.repo}/issues/new?title={title}&body={notes}\"\nvars = { repo = \"o/n\" }\n",
    );
    let r = router(&repo, Some(cfg.path()), None);
    let (status, body) = page(&r, "/_send/targets").await;
    assert_eq!(status, 200, "{body}");
    let want = serde_json::json!([{
        "id": "gh",
        "label": "New issue",
        "parts": [
            {"lit": "https://github.com/o/n/issues/new?title="},
            {"ph": "title"},
            {"lit": "&body="},
            {"ph": "notes"},
        ],
    }]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).expect("json"),
        want
    );
}
