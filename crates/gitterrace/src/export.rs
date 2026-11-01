//! `export`: render the site to static files by driving the router in-process.
//!
//! A full crawl of a real repository visits every commit and every blob, which
//! is tens of thousands of pages. The export is therefore scoped: the
//! overview, the insights pages, branches and tags, the newest N commits with
//! their log pages, and the file tree plus files at HEAD up to a size cap.
//! Links to anything outside that set are left pointing at their original path
//! and simply 404 in the exported copy, the same as a link to a page that was
//! never published.

use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use gitterrace_git::percent_encode;
use tower::ServiceExt;

/// What to include in an export.
pub struct Scope {
    /// Newest commits whose pages (and log pages) are written.
    pub commits: usize,
    /// Largest file, in bytes, whose page is written.
    pub max_file: u64,
}

/// Largest response body read from the router.
const MAX_BODY: usize = 64 * 1024 * 1024;
/// Hard ceiling on pages, so a mis-scoped export fails instead of filling a disk.
const MAX_PAGES: usize = 20_000;
/// Pages the crawl starts from, beside every page the server prewarms; each is exported
/// whatever links point at it.
const SEEDS: [&str; 2] = ["/refs", "/log/HEAD"];

/// Every seed: the extra ones, then the server's prewarmed pages (every unfiltered insights
/// page), so a page added to the server is exported without touching this list.
fn seeds() -> impl Iterator<Item = &'static str> {
    SEEDS.into_iter().chain(gitterrace_web::WARM_PAGES)
}

/// Writes the scoped site to `out` and returns how many files were written.
pub async fn export(repo_path: &Path, out: &Path, scope: &Scope) -> anyhow::Result<usize> {
    // Swapped arguments (`export <repo> <out>`) would scatter pages over a
    // checkout. An ignored folder inside a repo is fine; its root is not.
    if out.join(".git").exists() {
        bail!(
            "{} is the root of a git repository. Choose an output folder (usage: export <OUT> [REPO])",
            out.display()
        );
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let router = gitterrace_web::app_router(repo_path).context("building the router")?;
    wait_for_index(&router).await?;
    let repo = gitterrace_git::Repo::discover(repo_path).context("opening the repository")?;
    let head = repo.resolve("HEAD").context("resolving HEAD")?;
    let stamp = snapshot_label(&repo, &head)?;

    let mut queue: VecDeque<String> = VecDeque::new();
    for seed in seeds() {
        queue.push_back(seed.to_owned());
    }
    for c in repo
        .log(&head, None, 0, scope.commits)
        .context("reading the log")?
    {
        queue.push_back(format!("/commit/{}", c.id));
    }
    let allowed = Allowed::new(&repo, &head, scope)?;

    let mut seen: BTreeSet<String> = queue.iter().cloned().collect();
    let mut written = 0usize;
    // Pages render on blocking threads, so one at a time leaves the cores idle.
    // Fetch a batch concurrently, then write and follow links in queue order so
    // the output and the page cap stay deterministic.
    let width = std::thread::available_parallelism().map_or(4, |n| n.get() * 2);
    while !queue.is_empty() {
        let batch: Vec<String> = queue.drain(..queue.len().min(width)).collect();
        let mut tasks = tokio::task::JoinSet::new();
        for (i, path) in batch.into_iter().enumerate() {
            let router = router.clone();
            tasks.spawn(async move { (i, fetch(&router, &path).await.map(|r| (path, r))) });
        }
        let mut done = Vec::with_capacity(tasks.len());
        while let Some(joined) = tasks.join_next().await {
            done.push(joined.context("an export task panicked")?);
        }
        done.sort_by_key(|(i, _)| *i);
        for (_, fetched) in done {
            let (path, (status, content_type, body)) = fetched?;
            if status != StatusCode::OK {
                continue;
            }
            if written >= MAX_PAGES {
                bail!("export stopped at {MAX_PAGES} pages. Lower --commits or --max-file-kb");
            }
            let body = if content_type.starts_with("text/html") {
                mark_snapshot(&body, &stamp)
            } else {
                body
            };
            write_page(out, &path, &content_type, &body)?;
            written += 1;
            if content_type.starts_with("text/html") || content_type.starts_with("text/css") {
                for link in links(&String::from_utf8_lossy(&body)) {
                    if allowed.contains(&link) && seen.insert(link.clone()) {
                        queue.push_back(link);
                    }
                }
            }
        }
    }
    write_page(out, "/snapshot.css", "text/css", SNAPSHOT_CSS.as_bytes())?;
    Ok(written + 1)
}

/// Polls the progress endpoint so insights pages are exported built, not "indexing".
async fn wait_for_index(router: &Router) -> anyhow::Result<()> {
    eprintln!("building the cache (first run reads every commit once)...");
    for _ in 0..3_600 {
        let (_, _, body) = fetch(router, "/api/progress").await?;
        let ready = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("ready")?.as_bool())
            .unwrap_or(false);
        if ready {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    bail!("the cache did not finish building within 15 minutes")
}

async fn fetch(router: &Router, path: &str) -> anyhow::Result<(StatusCode, String, Vec<u8>)> {
    // The router refuses requests without a local Host (DNS-rebinding guard).
    // Paths travel decoded (they name files on disk); encode them for the URI.
    let req = Request::get(percent_encode(path, true))
        .header(axum::http::header::HOST, "localhost")
        .body(Body::empty())
        .context("building a request")?;
    let res = router
        .clone()
        .oneshot(req)
        .await
        .context("routing a request")?;
    let status = res.status();
    let content_type = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let body = to_bytes(res.into_body(), MAX_BODY)
        .await
        .with_context(|| format!("reading {path}"))?;
    Ok((status, content_type, body.to_vec()))
}

/// The set of paths an export follows.
struct Allowed {
    /// Exact paths allowed besides the prefix rules.
    exact: BTreeSet<String>,
    /// Commit ids in scope.
    commits: BTreeSet<String>,
    /// Paths (relative to the repo root) of files and directories at HEAD in scope.
    tree: BTreeSet<String>,
}

impl Allowed {
    fn new(repo: &gitterrace_git::Repo, head: &str, scope: &Scope) -> anyhow::Result<Self> {
        let commits = repo
            .log(head, None, 0, scope.commits)
            .context("reading the log")?
            .into_iter()
            .map(|c| c.id)
            .collect();
        let mut tree = BTreeSet::new();
        walk(repo, head, "", scope.max_file, &mut tree)?;
        let exact = seeds().map(str::to_owned).collect();
        Ok(Self {
            exact,
            commits,
            tree,
        })
    }

    fn contains(&self, link: &str) -> bool {
        if self.exact.contains(link) || link.starts_with("/static/") {
            return true;
        }
        if let Some(id) = link.strip_prefix("/commit/") {
            return self.commits.contains(id);
        }
        // Tree, file and raw pages are only exported at HEAD, the one revision
        // every "browse files" link in scope resolves to.
        for prefix in ["/tree/HEAD", "/blob/HEAD/", "/raw/HEAD/"] {
            if let Some(rest) = link.strip_prefix(prefix) {
                let rel = rest.trim_matches('/');
                return rel.is_empty() || self.tree.contains(rel);
            }
        }
        false
    }
}

fn walk(
    repo: &gitterrace_git::Repo,
    rev: &str,
    dir: &str,
    max_file: u64,
    out: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    for e in repo
        .tree(rev, dir)
        .with_context(|| format!("listing {dir}"))?
    {
        let path = if dir.is_empty() {
            e.name.clone()
        } else {
            format!("{dir}/{}", e.name)
        };
        match e.kind {
            gitterrace_git::TreeEntryKind::Dir => {
                out.insert(path.clone());
                walk(repo, rev, &path, max_file, out)?;
            }
            gitterrace_git::TreeEntryKind::File | gitterrace_git::TreeEntryKind::Symlink => {
                if e.size.unwrap_or(0) <= max_file {
                    out.insert(path);
                }
            }
            gitterrace_git::TreeEntryKind::Submodule => {}
        }
    }
    Ok(())
}

/// Same-origin absolute paths from `href`/`src` attributes and CSS `url()`s,
/// without query strings or fragments.
fn links(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for marker in ["href=\"", "src=\"", "url("] {
        let mut rest = html;
        while let Some(i) = rest.find(marker) {
            rest = &rest[i + marker.len()..];
            // `url('x')` and `url("x")` open with a quote; skip it before
            // looking for the closing delimiter.
            rest = rest.trim_start_matches(['"', '\'']);
            let end = rest.find(['"', ')', '\'']).unwrap_or(rest.len());
            let raw = &rest[..end];
            if raw.starts_with('/') && !raw.starts_with("//") {
                let path = raw.split(['?', '#']).next().unwrap_or(raw);
                out.push(decode_entities(path));
            }
        }
    }
    out
}

fn decode_entities(s: &str) -> String {
    let s = s
        .replace("&amp;", "&")
        .replace("&#x2F;", "/")
        .replace("&#47;", "/");
    percent_decode(&s)
}

/// Inverse of the web's `fmt::enc`: `%XX` back to bytes, then UTF-8. Links are
/// matched and written by their real name, so `a b.md` exports as `a b.md`, not
/// `a%20b.md`, which a static server would decode and then miss.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| char::from(c).to_digit(16);
        if b[i] == b'%'
            && let (Some(&h), Some(&l)) = (b.get(i + 1), b.get(i + 2))
            && let (Some(h), Some(l)) = (hex(h), hex(l))
        {
            out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Style for the snapshot banner and the static page, written next to the pages. Kept out of
/// the app's own CSS: only an export is ever static. Colours are the app's tokens, already on
/// the page. A static copy has no server, so the chrome that needs one is hidden: search, the
/// action buttons (their menu runs scripts and Send to), the working-tree chip and live notes.
/// Copy permalink stays: it needs nothing but the page.
const SNAPSHOT_CSS: &str = ".snapshot{background:var(--accent-soft);color:var(--fg);\
 border-bottom:1px solid var(--line-strong);font:var(--text-sm)/1.4 var(--sans);padding:var(--space-2) 0}\
 .snapshot a{color:var(--accent-fg);font-weight:var(--weight-semibold)}\
 body[data-static] .topsearch,body[data-static] .actbtn,body[data-static] .wtchip,\
 body[data-static] .livenote,body[data-static] .actnote{display:none!important}\n";

/// What the snapshot banner names: the tag on HEAD when there is one, else the short commit id.
fn snapshot_label(repo: &gitterrace_git::Repo, head: &str) -> anyhow::Result<String> {
    let tag = repo
        .refs()
        .context("reading refs")?
        .into_iter()
        .filter(|r| r.kind == gitterrace_git::RefKind::Tag && r.target == head)
        .map(|r| r.name)
        .max();
    Ok(tag.unwrap_or_else(|| gitterrace_git::short_id(head).to_owned()))
}

/// The banner every exported page carries, so nobody mistakes a frozen copy for a live view:
/// it says what it is, which revision it froze, and where the real thing comes from. Inserted
/// right after `<body ...>`, outside the app's chrome, so no template has to know about export.
fn mark_snapshot(html: &[u8], label: &str) -> Vec<u8> {
    let page = String::from_utf8_lossy(html);
    let Some(open) = page.find("<body") else {
        return html.to_vec();
    };
    let Some(end) = page[open..].find('>').map(|i| open + i + 1) else {
        return html.to_vec();
    };
    let banner = format!(
        "<link rel=\"stylesheet\" href=\"/snapshot.css\">\
         <div class=\"snapshot\" role=\"note\"><div class=\"wrap\">Static snapshot at <b>{}</b>. \
         Search, live updates and actions need the real thing. \
         <a href=\"https://gitterrace.dev\">Install gitterrace</a></div></div>",
        escape(label)
    );
    let mut out = String::with_capacity(page.len() + banner.len() + 12);
    // `data-static` tells the page scripts there is no server: they skip live updates and
    // the actions poll, and the snapshot CSS hides what needs one.
    out.push_str(&page[..end - 1]);
    out.push_str(" data-static>");
    out.push_str(&banner);
    out.push_str(&page[end..]);
    out.into_bytes()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// File the page lands in: HTML pages become `<path>/index.html` so their links
/// resolve when the folder is served by any static server.
fn target(out: &Path, path: &str, content_type: &str) -> PathBuf {
    let rel = path.trim_start_matches('/');
    let mut p = out.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty() && *s != "..") {
        p.push(seg);
    }
    if content_type.starts_with("text/html") {
        p.push("index.html");
    }
    p
}

fn write_page(out: &Path, path: &str, content_type: &str, body: &[u8]) -> anyhow::Result<()> {
    let file = target(out, path, content_type);
    let root = out
        .canonicalize()
        .with_context(|| format!("resolving {}", out.display()))?;
    // `target` drops `..`, but a symlink already inside `out` could still lead
    // elsewhere. Walk down one component at a time and refuse before creating
    // anything: checking after `create_dir_all` would already have written
    // directories through the link.
    let mut dir = out.to_path_buf();
    if let Some(parent) = file.parent() {
        for part in parent.strip_prefix(out).unwrap_or(parent).components() {
            dir.push(part);
            if !dir.exists() {
                std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
            }
            let real = dir
                .canonicalize()
                .with_context(|| format!("resolving {}", dir.display()))?;
            anyhow::ensure!(
                real.starts_with(&root),
                "refusing to write {} outside {}",
                file.display(),
                out.display()
            );
        }
    }
    // The parents are real directories inside `out`; the leaf itself may still be a link.
    if let Ok(m) = std::fs::symlink_metadata(&file) {
        anyhow::ensure!(
            m.is_file(),
            "refusing to write {}: not a regular file",
            file.display()
        );
    }
    std::fs::write(&file, body).with_context(|| format!("writing {}", file.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gitterrace_git::percent_encode;

    #[cfg(unix)]
    use crate::export::write_page;
    use crate::export::{links, percent_decode, target};

    #[test]
    fn encoded_names_round_trip_to_real_file_names() {
        let html = r#"<a href="/blob/HEAD/docs/a%20b%23c.md">x</a><a href="/blob/HEAD/%C3%BCber.md">y</a>"#;
        assert_eq!(
            links(html),
            vec!["/blob/HEAD/docs/a b#c.md", "/blob/HEAD/über.md"]
        );
        for p in [
            "/blob/HEAD/docs/a b#c.md",
            "/blob/HEAD/über.md",
            "/tree/HEAD/x%y",
        ] {
            assert_eq!(percent_decode(&percent_encode(p, true)), p);
        }
        assert_eq!(
            percent_encode("/blob/HEAD/a b.md", true),
            "/blob/HEAD/a%20b.md"
        );
        assert_eq!(percent_decode("bad%zz%4"), "bad%zz%4");
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
    fn html_pages_become_index_files_and_dotdot_is_dropped() {
        let out = Path::new("/o");
        assert_eq!(
            target(out, "/", "text/html; charset=utf-8"),
            Path::new("/o/index.html")
        );
        assert_eq!(
            target(out, "/stats/files", "text/html"),
            Path::new("/o/stats/files/index.html")
        );
        assert_eq!(
            target(out, "/static/app.css", "text/css"),
            Path::new("/o/static/app.css")
        );
        assert_eq!(
            target(out, "/raw/HEAD/../../etc", "text/plain"),
            Path::new("/o/raw/HEAD/etc")
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_page_refuses_a_symlink_that_leads_outside() {
        let out = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink(elsewhere.path(), out.path().join("stats")).expect("symlink");
        let err = write_page(out.path(), "/stats/files", "text/html", b"x").unwrap_err();
        assert!(err.to_string().contains("refusing to write"), "{err}");
        assert!(!elsewhere.path().join("files").exists());
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

    #[tokio::test]
    async fn export_refuses_a_repository_root_as_output() {
        let repo = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(repo.path().join(".git")).expect("mkdir");
        let scope = crate::export::Scope {
            commits: 1,
            max_file: 1024,
        };
        let err = crate::export::export(repo.path(), repo.path(), &scope)
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
        };
        crate::export::export(repo.path(), &out, &scope)
            .await
            .expect("export");
        for page in gitterrace_web::WARM_PAGES {
            let file = target(&out, page, "text/html");
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
}
