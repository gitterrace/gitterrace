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

/// Writes the scoped site to `out` and returns how many files were written.
pub async fn export(repo_path: &Path, out: &Path, scope: &Scope) -> anyhow::Result<usize> {
    let router = gitterrace_web::app_router(repo_path).context("building the router")?;
    wait_for_index(&router).await?;
    let repo = gitterrace_git::Repo::discover(repo_path).context("opening the repository")?;
    let head = repo.resolve("HEAD").context("resolving HEAD")?;

    let mut queue: VecDeque<String> = VecDeque::new();
    for seed in [
        "/",
        "/refs",
        "/stats",
        "/stats/files",
        "/stats/types",
        "/log/HEAD",
    ] {
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
    while let Some(path) = queue.pop_front() {
        if written >= MAX_PAGES {
            bail!("export stopped at {MAX_PAGES} pages; lower --commits or --max-file-kb");
        }
        let (status, content_type, body) = fetch(&router, &path).await?;
        if status != StatusCode::OK {
            continue;
        }
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
    Ok(written)
}

/// Polls the progress endpoint so insights pages are exported built, not "indexing".
async fn wait_for_index(router: &Router) -> anyhow::Result<()> {
    eprintln!("building the cache (first run reads every commit once)...");
    for _ in 0..3_600 {
        let (_, _, body) = fetch(router, "/api/progress").await?;
        if String::from_utf8_lossy(&body).contains("\"ready\":true") {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    bail!("the cache did not finish building within 15 minutes")
}

async fn fetch(router: &Router, path: &str) -> anyhow::Result<(StatusCode, String, Vec<u8>)> {
    let req = Request::get(path)
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
        let exact = [
            "/",
            "/refs",
            "/stats",
            "/stats/files",
            "/stats/types",
            "/log/HEAD",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
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
    s.replace("&amp;", "&")
        .replace("&#x2F;", "/")
        .replace("&#47;", "/")
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
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&file, body).with_context(|| format!("writing {}", file.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::export::{links, target};

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
}
