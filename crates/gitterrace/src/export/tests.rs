use std::path::Path;

#[cfg(unix)]
use crate::export::write_page;
use crate::export::{links, mark_snapshot, mark_snapshot_as, target};

/// The budget an export gets with no flag and no config: `limits.export_pages`'s default.
const DEFAULT_PAGES: usize = 20_000;

#[test]
fn headers_file_carries_the_page_policy() {
    let h = super::headers_file();
    assert!(h.contains(gitterrace_web::PAGE_CSP));
    assert!(!h.contains("script-src 'self' 'unsafe-inline'"));
}

/// Guard class: a static host (Cloudflare, Netlify) takes one `*` per rule and ignores a
/// rule with two, so a raw file would lose its sandbox. Every rule path has at most one.
#[test]
fn every_header_rule_has_at_most_one_splat() {
    let h = super::headers_file();
    let rules: Vec<&str> = h.lines().filter(|l| l.starts_with('/')).collect();
    assert!(rules.iter().any(|r| r.contains("/raw/")), "{h}");
    for r in rules {
        assert!(r.matches('*').count() <= 1, "{r} has more than one *");
    }
}

#[test]
fn encoded_names_round_trip_to_real_file_names() {
    let html =
        r#"<a href="/blob/HEAD/docs/a%20b%23c.md">x</a><a href="/blob/HEAD/%C3%BCber.md">y</a>"#;
    assert_eq!(
        links(html),
        vec!["/blob/HEAD/docs/a b#c.md", "/blob/HEAD/über.md"]
    );
}

#[test]
fn links_keeps_same_origin_paths_only() {
    let html = r#"<a href="/commit/abc?x=1#L2">c</a><a href="https://x.dev/">e</a>
            <a href="//cdn.x/a.js">p</a><img src="/raw/HEAD/a.png"><style>b{background:url('/static/f.woff2')}</style>
            <a href="/tree/HEAD/a&amp;b">t</a>"#;
    assert_eq!(
        links(html),
        vec![
            "/commit/abc",
            "/tree/HEAD/a&b",
            "/raw/HEAD/a.png",
            "/static/f.woff2"
        ]
    );
}

#[test]
fn html_pages_become_index_files() {
    let out = Path::new("/o");
    let t = |p, ct| target(out, p, ct).expect("plain path");
    assert_eq!(
        t("/", "text/html; charset=utf-8"),
        Path::new("/o/index.html")
    );
    assert_eq!(
        t("/stats/hotspots", "text/html"),
        Path::new("/o/stats/hotspots/index.html")
    );
    assert_eq!(
        t("/static/app.css", "text/css"),
        Path::new("/o/static/app.css")
    );
}

/// Class: any segment that is not a plain name. Whatever a path holds, a target is either
/// refused or lies under `out` with nothing but Normal components below it.
#[test]
fn target_refuses_every_hostile_segment() {
    let out = Path::new("/o");
    let hostile = [
        "/raw/HEAD/../../etc",
        "/..",
        "/./x",
        "/a/./b",
        "/blob/HEAD/a\\..\\..\\x",
        "/blob/HEAD/C:/x",
        "/blob/HEAD/c:x",
        "/blob/HEAD/a\0b",
        "/blob/HEAD/a/../../b",
    ];
    for path in hostile {
        assert_eq!(
            target(out, path, "text/plain"),
            None,
            "{path:?} was accepted"
        );
    }
    for path in hostile
        .iter()
        .copied()
        .chain(["/a b/c", "//x//y/", "/über"])
    {
        for ct in ["text/html", "text/plain"] {
            if let Some(t) = target(out, path, ct) {
                let below = t.strip_prefix(out).expect("under out");
                assert!(
                    below
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_))),
                    "{path:?} -> {}",
                    t.display()
                );
            }
        }
    }
}

#[test]
fn snapshot_names_the_gitterrace_version() {
    let html = b"<html><head><title>t</title></head><body><main>m</main></body></html>";
    let out = String::from_utf8(mark_snapshot(html, "v1", None)).expect("utf8");
    let v = env!("GITTERRACE_VERSION");
    assert!(out.contains(&format!("Made with gitterrace {v}.")), "{out}");
    assert!(
        out.contains(&format!(
            "<meta name=\"generator\" content=\"gitterrace {v}\"></head>"
        )),
        "{out}"
    );
}

/// Guard class: every value the banner interpolates is escaped, the version included.
#[test]
fn snapshot_escapes_the_version() {
    let html = b"<html><head></head><body><main>m</main></body></html>";
    let out = String::from_utf8(mark_snapshot_as(html, "v1", None, "1<b>&\"x")).expect("utf8");
    assert!(!out.contains("1<b>"), "{out}");
    assert!(
        out.contains("Made with gitterrace 1&lt;b&gt;&amp;&quot;x."),
        "{out}"
    );
    assert!(
        out.contains("content=\"gitterrace 1&lt;b&gt;&amp;&quot;x\""),
        "{out}"
    );
}

#[test]
fn mark_snapshot_anchors_on_the_body_element_after_head() {
    let html = b"<html><head><title>&lt;body</title><meta content=\"<bodyx>\"></head>\
            <body class=\"p\"><main>m</main></body></html>";
    let out = String::from_utf8(mark_snapshot(html, "v1", None)).expect("utf8");
    assert!(out.contains("<title>&lt;body</title>"), "{out}");
    assert!(out.contains("content=\"<bodyx>\""), "{out}");
    assert!(out.contains("<body class=\"p\" data-static><link"), "{out}");
}

#[cfg(unix)]
#[test]
fn write_page_refuses_a_symlink_that_leads_outside() {
    let out = tempfile::tempdir().expect("tempdir");
    let elsewhere = tempfile::tempdir().expect("tempdir");
    std::os::unix::fs::symlink(elsewhere.path(), out.path().join("stats")).expect("symlink");
    let err = write_page(out.path(), "/stats/hotspots", "text/html", b"x").unwrap_err();
    assert!(err.to_string().contains("refusing to write"), "{err}");
    assert!(!elsewhere.path().join("hotspots").exists());
    write_page(out.path(), "/refs", "text/html", b"ok").expect("normal page writes");
    assert!(out.path().join("refs/index.html").exists());
}

#[cfg(unix)]
#[test]
fn write_page_refuses_a_symlink_at_the_leaf() {
    let out = tempfile::tempdir().expect("tempdir");
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let victim = elsewhere.path().join("victim");
    std::fs::write(&victim, "keep").expect("write");
    std::fs::create_dir(out.path().join("refs")).expect("mkdir");
    std::os::unix::fs::symlink(&victim, out.path().join("refs/index.html")).expect("symlink");
    let err = write_page(out.path(), "/refs", "text/html", b"x").unwrap_err();
    assert!(err.to_string().contains("refusing to write"), "{err}");
    assert_eq!(std::fs::read_to_string(&victim).expect("read"), "keep");
}

fn scope(force: bool) -> crate::export::Scope {
    crate::export::Scope {
        commits: 3,
        max_file: 1024,
        projects: Vec::new(),
        force,
        config: gitterrace_config::Config::default(),
    }
}

/// Class guard: export never writes into or clears a folder it did not create, unless
/// forced.
#[tokio::test]
async fn export_never_writes_into_or_clears_a_folder_it_did_not_create_unless_forced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = gitterrace_testkit::Fixture::standard(&dir.path().join("r"));
    let repos = [repo.path().to_path_buf()];
    let out = dir.path().join("foreign");
    std::fs::create_dir(&out).expect("mkdir");
    std::fs::write(out.join("mine.txt"), "keep").expect("write");
    let err = crate::export::export(&repos, &out, &scope(false), DEFAULT_PAGES)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not empty"), "{err}");
    assert_eq!(std::fs::read_dir(&out).expect("read").count(), 1);
    crate::export::export(&repos, &out, &scope(true), DEFAULT_PAGES)
        .await
        .expect("forced");
    assert!(
        out.join("mine.txt").is_file(),
        "force writes over, not clears"
    );
    assert!(out.join(".gitterrace-export").is_file());
    assert!(out.join("index.html").is_file());
}

#[tokio::test]
async fn re_export_removes_stale_pages_and_ignores_symlink_targets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = gitterrace_testkit::Fixture::standard(&dir.path().join("r"));
    let repos = [repo.path().to_path_buf()];
    let out = dir.path().join("out");
    crate::export::export(&repos, &out, &scope(false), DEFAULT_PAGES)
        .await
        .expect("first");
    std::fs::create_dir_all(out.join("old/page")).expect("mkdir");
    std::fs::write(out.join("old/page/index.html"), "stale").expect("write");
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).expect("mkdir");
    std::fs::write(outside.join("precious"), "keep").expect("write");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, out.join("link")).expect("symlink");
    crate::export::export(&repos, &out, &scope(false), DEFAULT_PAGES)
        .await
        .expect("second");
    assert!(!out.join("old").exists(), "stale page survived");
    assert!(!out.join("link").exists());
    assert!(outside.join("precious").is_file(), "symlink was followed");
    assert!(out.join("index.html").is_file());
}

#[tokio::test]
async fn default_folder_is_used_when_none_is_named() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = gitterrace_testkit::Fixture::standard(&dir.path().join("r"));
    let out = dir.path().join(crate::export::DEFAULT_OUT);
    crate::export::export(
        &[repo.path().to_path_buf()],
        &out,
        &scope(false),
        DEFAULT_PAGES,
    )
    .await
    .expect("export");
    assert!(out.join("index.html").is_file());
    assert_eq!(crate::export::DEFAULT_OUT, "gitterrace-export");
}

#[tokio::test]
async fn export_refuses_a_repository_root_as_output() {
    let repo = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(repo.path().join(".git")).expect("mkdir");
    let scope = crate::export::Scope {
        commits: 1,
        max_file: 1024,
        projects: Vec::new(),
        force: false,
        config: gitterrace_config::Config::default(),
    };
    let err = crate::export::export(
        &[repo.path().to_path_buf()],
        repo.path(),
        &scope,
        DEFAULT_PAGES,
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("root of a git repository"),
        "{err}"
    );
    let left: Vec<_> = std::fs::read_dir(repo.path()).expect("read").collect();
    assert_eq!(left.len(), 1, "nothing but .git may be written");
}

/// A static copy must hold every page the server treats as a landing page: the demo is
/// built from export, and a missing insights page is a dead tab, not a slow one.
#[tokio::test]
async fn export_writes_every_prewarmed_page() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = gitterrace_testkit::Fixture::standard(&dir.path().join("r"));
    let out = dir.path().join("out");
    let scope = crate::export::Scope {
        commits: 5,
        max_file: 64 * 1024,
        projects: Vec::new(),
        force: false,
        config: gitterrace_config::Config::default(),
    };
    crate::export::export(&[repo.path().to_path_buf()], &out, &scope, DEFAULT_PAGES)
        .await
        .expect("export");
    let root =
        std::fs::read_to_string(target(&out, "/", "text/html").expect("root")).expect("root page");
    assert!(
        root.contains("url=r/r/index.html"),
        "/ forwards to the one repository: {root}"
    );
    for page in gitterrace_web::WARM_PAGES {
        let page = format!("/r/r{page}");
        let file = target(&out, &page, "text/html").expect("plain path");
        assert!(
            file.is_file(),
            "{page} was not exported (looked for {})",
            file.display()
        );
        let html = std::fs::read_to_string(&file).expect("read page");
        assert!(
            html.contains("class=\"snapshot\"") && html.contains("Static snapshot at"),
            "{page} does not say it is a static snapshot"
        );
    }
    assert!(
        out.join("snapshot.css").is_file(),
        "the banner style is missing"
    );
}

/// Several repositories export like the server serves them: an index at `/`, each one
/// under `/r/<name>/` with every insights page, and no link leaves its own repository.
#[tokio::test]
async fn several_repositories_export_like_the_server_serves_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = gitterrace_testkit::Fixture::standard(&dir.path().join("alpha"));
    let b = gitterrace_testkit::Fixture::standard(&dir.path().join("beta"));
    let out = dir.path().join("out");
    let scope = crate::export::Scope {
        commits: 5,
        max_file: 64 * 1024,
        projects: Vec::new(),
        force: false,
        config: gitterrace_config::Config::default(),
    };
    crate::export::export(
        &[a.path().to_path_buf(), b.path().to_path_buf()],
        &out,
        &scope,
        DEFAULT_PAGES,
    )
    .await
    .expect("export");
    let index = std::fs::read_to_string(out.join("index.html")).expect("index page");
    for name in ["alpha", "beta"] {
        assert!(
            index.contains(&format!("/r/{name}/")),
            "index does not link {name}"
        );
        for page in gitterrace_web::WARM_PAGES {
            let file = target(&out, &format!("/r/{name}{page}"), "text/html").expect("plain path");
            assert!(file.is_file(), "/r/{name}{page} was not exported");
        }
    }
    // A repository's pages link only into itself, the index, or the shared assets.
    let alpha =
        std::fs::read_to_string(target(&out, "/r/alpha/", "text/html").expect("plain path"))
            .expect("alpha");
    // Written for disk, a page holds no root-absolute link at all.
    assert!(
        links(&alpha).is_empty(),
        "absolute links: {:?}",
        links(&alpha)
    );
}

/// Class: a repository file a browser would run scripts in. A static host serves exported
/// bytes by extension without the server's sandbox, so every file under any raw/ directory
/// must be one `plain_media` allows. Driven by `plain_media` itself, so a scriptable type
/// added to media.rs is covered without touching this test.
#[tokio::test]
async fn export_ships_no_scriptable_raw_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let names = ["evil.svg", "x.html", "x.xhtml", "page.pdf", "a.png"];
    let readme = names
        .map(|n| format!("[{n}](/raw/HEAD/{n}) ![{n}](/raw/HEAD/{n})\n"))
        .concat();
    let mut fx = gitterrace_testkit::Fixture::init(&dir.path().join("r"))
        .write("README.md", &readme)
        .write(
            "evil.svg",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
        )
        .write("x.html", "<script>alert(1)</script>")
        .write(
            "x.xhtml",
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><script>alert(1)</script></html>",
        )
        .write("page.pdf", "%PDF-1.4");
    // A minimal PNG signature: what matters is the name, but keep the bytes honest.
    std::fs::write(fx.path().join("a.png"), b"\x89PNG\r\n\x1a\n").expect("png");
    fx = fx.commit("files");
    let out = dir.path().join("out");
    let scope = crate::export::Scope {
        commits: 5,
        max_file: 64 * 1024,
        projects: Vec::new(),
        force: false,
        config: gitterrace_config::Config::default(),
    };
    crate::export::export(&[fx.path().to_path_buf()], &out, &scope, DEFAULT_PAGES)
        .await
        .expect("export");
    let mut raw = Vec::new();
    let mut stack = vec![out.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read_dir") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .strip_prefix(&out)
                .expect("under out")
                .components()
                .any(|c| c.as_os_str() == "raw")
            {
                raw.push(p);
            }
        }
    }
    for p in &raw {
        assert!(
            gitterrace_web::plain_media(&p.to_string_lossy()),
            "scriptable raw file exported: {}",
            p.display()
        );
    }
    assert!(
        out.join("r/r/raw/HEAD/a.png").is_file(),
        "inert media must still export; found {raw:?}"
    );
}

/// A folded diff fragment is exported for an in-scope commit and a plain index only.
#[test]
fn diff_fragments_follow_the_commit_scope() {
    let allowed = super::Allowed {
        commits: std::iter::once("abc".to_owned()).collect(),
        tree: std::collections::BTreeSet::new(),
        seeds: Vec::new(),
    };
    assert!(allowed.contains("/diff/abc/0"));
    assert!(allowed.contains("/diff/abc/1999"));
    for bad in [
        "/diff/other/0",
        "/diff/abc/x",
        "/diff/abc/",
        "/diff/abc/+1",
        "/diff/abc/1/../x",
        "/diff/abc/1/2",
    ] {
        assert!(!allowed.contains(bad), "{bad}");
    }
    assert!(super::is_fragment("/r/name/diff/abc/0") && super::is_fragment("/diff/abc/0"));
    assert!(!super::is_fragment("/r/name/commit/abc") && !super::is_fragment("/r/diff/commit/abc"));
}

/// An export serves what the server would: with insights and the credit line switched off it
/// writes no insights page and no credit footer, and no written page links either.
#[tokio::test]
async fn export_skips_what_features_turn_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = gitterrace_testkit::Fixture::standard(&dir.path().join("r"));
    let out = dir.path().join("out");
    let config =
        gitterrace_config::parse("[features]\ninsights_enabled = false\ncredit_enabled = false\n")
            .expect("parse")
            .config;
    let scope = crate::export::Scope {
        commits: 5,
        max_file: 64 * 1024,
        projects: Vec::new(),
        force: false,
        config,
    };
    crate::export::export(&[repo.path().to_path_buf()], &out, &scope, DEFAULT_PAGES)
        .await
        .expect("export");
    assert!(!out.join("r/r/stats").exists(), "insights exported");
    let mut seen = 0;
    let mut todo = vec![out.clone()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if p.extension().is_some_and(|x| x == "html") {
                let html = std::fs::read_to_string(&p).expect("read");
                assert!(
                    !html.contains("/r/r/stats"),
                    "{} links insights",
                    p.display()
                );
                assert!(
                    !html.contains("snapshot credit"),
                    "{} has a credit",
                    p.display()
                );
                seen += 1;
            }
        }
    }
    assert!(seen > 3, "pages were written");
}
