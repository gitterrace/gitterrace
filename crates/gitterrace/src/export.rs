//! `export`: render the site to static files by driving the router in-process.
//!
//! A full crawl of a real repository visits every commit and every blob, which
//! is tens of thousands of pages. The export is therefore scoped: the
//! overview, the insights pages, branches and tags, the newest N commits with
//! their log pages, and the file tree plus files at HEAD up to a size cap.
//! Links to anything outside that set are left pointing at their original path
//! and 404 in the exported copy, the same as a link to a page that was
//! never published.

mod snapshot;
mod write;

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
use gitterrace_git::{percent_decode, percent_encode_path};
use tower::ServiceExt;

#[cfg(test)]
use crate::export::snapshot::mark_snapshot_as;
use crate::export::snapshot::{SNAPSHOT_CSS, mark_snapshot, snapshot_label};
#[cfg(test)]
use crate::export::write::target;
use crate::export::write::{write_leaf, write_page};

/// What to include in an export.
pub(crate) struct Scope {
    /// Newest commits whose pages (and log pages) are written.
    pub(crate) commits: usize,
    /// Largest file, in bytes, whose page is written.
    pub(crate) max_file: u64,
    /// Repositories that are someone else's project, credited in their banner by folder name.
    pub(crate) projects: Vec<Project>,
    /// Write into a non-empty folder that no earlier export made.
    pub(crate) force: bool,
}

/// Someone else's repository shown in an export, named and linked in the banner.
pub(crate) struct Project {
    pub(crate) name: String,
    pub(crate) url: String,
}

impl Project {
    /// `name=url`, with an http(s) URL: the banner links it, so nothing else may get in.
    pub(crate) fn parse(s: &str) -> anyhow::Result<Self> {
        let (name, url) = s
            .split_once('=')
            .context("--project takes NAME=URL, for example jq=https://github.com/jqlang/jq")?;
        if name.trim().is_empty() || !(url.starts_with("https://") || url.starts_with("http://")) {
            bail!("--project takes NAME=URL with an http(s) URL, got {s:?}");
        }
        Ok(Self {
            name: name.trim().to_owned(),
            url: url.to_owned(),
        })
    }
}

/// Largest response body read from the router.
const MAX_BODY: usize = 64 * 1024 * 1024;
/// Hard ceiling on pages, so a wrongly scoped export fails instead of filling a disk.
const MAX_PAGES: usize = 20_000;
/// Pages the crawl starts from, beside every page the server prewarms; each is exported
/// whatever links point at it.
const SEEDS: [&str; 2] = ["/refs", "/log/HEAD"];

/// Every seed: the extra ones, then the server's prewarmed pages (every unfiltered insights
/// page), so a page added to the server is exported without touching this list.
fn seeds() -> impl Iterator<Item = &'static str> {
    SEEDS.into_iter().chain(gitterrace_web::WARM_PAGES)
}

/// Where an export goes when no folder is named.
pub(crate) const DEFAULT_OUT: &str = "gitterrace-export";
/// Written into every export folder; its presence says an export made the folder.
const MARKER: &str = ".gitterrace-export";

/// Gets `out` ready: created if missing; a folder with the marker is emptied so no stale page
/// survives; a non-empty folder without it is refused unless `force`.
fn prepare_out(repos: &[PathBuf], out: &Path, force: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let marked = out.join(MARKER).is_file();
    let empty = std::fs::read_dir(out)
        .with_context(|| format!("reading {}", out.display()))?
        .next()
        .is_none();
    if !marked && !empty && !force {
        bail!(
            "{} is not empty and was not made by an export (no {MARKER} file); \
             choose another folder or pass --force to write into it",
            out.display()
        );
    }
    if marked {
        // Clearing deletes, so it must never reach a served repository or the working folder.
        let root = out.canonicalize().context("resolving the output folder")?;
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|d| d.canonicalize().ok());
        let inside = repos
            .iter()
            .filter_map(|r| r.canonicalize().ok())
            .chain(cwd)
            .any(|p| p.starts_with(&root));
        if inside {
            bail!(
                "{} holds a repository or the current folder. It cannot be cleared for a fresh export",
                out.display()
            );
        }
        clear(out)?;
    }
    let names: Vec<String> = repos
        .iter()
        .map(|r| {
            r.file_name().map_or_else(
                || r.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let line = format!(
        "gitterrace {} {}\n",
        env!("GITTERRACE_VERSION"),
        names.join(",")
    );
    write_leaf(&out.join(MARKER), line.as_bytes())?;
    exclude_from_worktree(out);
    Ok(())
}

/// An export written straight into a worktree (`export` run in a checkout) would show as
/// untracked; list it in that clone's `.git/info/exclude`, which is local and never committed.
/// Only a folder directly under the worktree root qualifies; anything else is left alone.
#[expect(
    clippy::print_stderr,
    reason = "a CLI note: the user sees why their exclude file changed"
)]
fn exclude_from_worktree(out: &Path) {
    let Ok(out) = out.canonicalize() else { return };
    let (Some(parent), Some(name)) = (out.parent(), out.file_name()) else {
        return;
    };
    let Ok(repo) = gitterrace_git::Repo::discover(parent) else {
        return;
    };
    if repo.root() != parent {
        return;
    }
    let name = name.to_string_lossy();
    match repo.exclude_locally(&name) {
        Ok(true) => eprintln!("listed /{name}/ in .git/info/exclude so the export stays untracked"),
        Ok(false) => {}
        Err(e) => eprintln!("could not list /{name}/ in .git/info/exclude: {e}"),
    }
}

/// Removes everything inside `dir`. A symlink is removed as a link, never followed.
fn clear(dir: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let kind = std::fs::symlink_metadata(&path)?.file_type();
        let done = if kind.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        done.with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// Writes the scoped site to `out` and returns how many files were written. One repository
/// exports at `/`; several export like the server serves them: an index at `/` and each
/// repository under `/r/<name>/`, named by the server's own table.
pub(crate) async fn export(repos: &[PathBuf], out: &Path, scope: &Scope) -> anyhow::Result<usize> {
    // Swapped arguments (`export <repo> <out>`) would scatter pages over a
    // checkout. An ignored folder inside a repo is fine; its root is not.
    if out.join(".git").exists() {
        bail!(
            "{} is the root of a git repository. Choose an output folder (usage: export <OUT> [REPO])",
            out.display()
        );
    }
    prepare_out(repos, out, scope.force)?;
    let router = gitterrace_web::multi_router(repos.to_vec()).context("building the router")?;
    let sites = sites(&router, repos).await?;

    let mut queue: VecDeque<String> = root_page(out, &sites)?.into_iter().collect();
    let mut site_of = Vec::with_capacity(sites.len());
    for (repo_path, site) in repos.iter().zip(&sites) {
        wait_for_index(&router, site).await?;
        let repo = gitterrace_git::Repo::discover(repo_path).context("opening the repository")?;
        let head = repo.resolve("HEAD").context("resolving HEAD")?;
        for seed in seeds() {
            queue.push_back(format!("{site}{seed}"));
        }
        for c in repo
            .log(&head, None, 0, scope.commits)
            .context("reading the log")?
        {
            queue.push_back(format!("{site}/commit/{}", c.id));
        }
        let name = repo.name();
        site_of.push(Site {
            prefix: site.clone(),
            stamp: snapshot_label(&repo, &head)?,
            credit: scope.projects.iter().position(|p| p.name == name),
            allowed: Allowed::new(&repo, &head, scope)?,
        });
    }
    if let Some(p) = scope.projects.iter().find(|p| {
        !site_of
            .iter()
            .any(|s| s.credit.is_some_and(|i| scope.projects[i].name == p.name))
    }) {
        bail!(
            "--project {}: no exported repository has that folder name",
            p.name
        );
    }
    // The longest prefix wins, so `/r/a-2/` never matches `/r/a`. The root site (one repo, or
    // the index of several) has the empty prefix and catches the rest.
    site_of.sort_by_key(|s| std::cmp::Reverse(s.prefix.len()));
    let site_for = |path: &str| {
        site_of.iter().find(|s| {
            s.prefix.is_empty() || path == s.prefix || path.starts_with(&format!("{}/", s.prefix))
        })
    };

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
            let site = site_for(&path);
            let body = if content_type.starts_with("text/html") {
                // The index of several repositories names no revision.
                let stamp = site.map_or("", |s| s.stamp.as_str());
                let credit = site.and_then(|s| s.credit).map(|i| &scope.projects[i]);
                mark_snapshot(&body, stamp, credit)
            } else {
                body
            };
            written += usize::from(write_page(out, &path, &content_type, &body)?);
            if content_type.starts_with("text/html") || content_type.starts_with("text/css") {
                for link in links(&String::from_utf8_lossy(&body)) {
                    let ok = link.starts_with("/static/")
                        || site_for(&link).is_some_and(|s| s.allows(&link));
                    if ok && seen.insert(link.clone()) {
                        queue.push_back(link);
                    }
                }
            }
        }
    }
    write_page(out, "/snapshot.css", "text/css", SNAPSHOT_CSS.as_bytes())?;
    write_page(out, "/_headers", "text/plain", headers_file().as_bytes())?;
    Ok(written + 2)
}

/// The root of the export: the index of several, queued to fetch, or for one repository a page
/// written now that forwards to it, since a static host cannot replay the server's redirect.
fn root_page(out: &Path, sites: &[String]) -> anyhow::Result<Option<String>> {
    if let [one] = sites {
        write_page(out, "/", "text/html", forward_page(one).as_bytes())?;
        return Ok(None);
    }
    Ok(Some("/".to_owned()))
}

/// The root page of a one-repository export: forwards to its pages. A meta refresh and a
/// link, since the page policy refuses inline script.
fn forward_page(prefix: &str) -> String {
    let to = gitterrace_web::escape_html(&format!("{prefix}/"));
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"refresh\" content=\"0; url={to}\"><title>gitterrace</title></head>\
         <body><a href=\"{to}\">{to}</a></body></html>\n"
    )
}

/// One repository in an export: where its pages live and which of them are in scope.
struct Site {
    /// `/r/<name>`, as the server names it.
    prefix: String,
    /// The revision the banner names.
    stamp: String,
    /// Index into `Scope::projects` when this repository is credited.
    credit: Option<usize>,
    allowed: Allowed,
}

impl Site {
    fn allows(&self, link: &str) -> bool {
        let local = link.strip_prefix(&self.prefix).unwrap_or(link);
        let local = if local.is_empty() { "/" } else { local };
        self.allowed.contains(local)
    }
}

/// Where each repository's pages live, in `repos` order, from the server itself: under the
/// names `/api/repos` lists, one repository too. Asking the router keeps naming (and its
/// collision rule) in one place.
async fn sites(router: &Router, repos: &[PathBuf]) -> anyhow::Result<Vec<String>> {
    let (_, _, body) = fetch(router, "/api/repos").await?;
    let items: Vec<serde_json::Value> =
        serde_json::from_slice(&body).context("reading /api/repos")?;
    let prefixes: Vec<String> = items
        .iter()
        .filter_map(|i| i.get("href")?.as_str())
        .map(|h| h.trim_end_matches('/').to_owned())
        .collect();
    if prefixes.len() != repos.len() {
        bail!(
            "the server lists {} repositories, expected {}",
            prefixes.len(),
            repos.len()
        );
    }
    Ok(prefixes)
}

#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// Polls the progress endpoint so insights pages are exported built, not "indexing".
async fn wait_for_index(router: &Router, prefix: &str) -> anyhow::Result<()> {
    eprintln!("building the cache for {prefix} (first run reads every commit once)...");
    for _ in 0..3_600 {
        let (_, _, body) = fetch(router, &format!("{prefix}/api/progress")).await?;
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
    let req = Request::get(percent_encode_path(path))
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

/// Host headers for a static copy: the app's own page policy, since the pages are its HTML,
/// so the two cannot drift. Raw files are sandboxed.
fn headers_file() -> String {
    let page = gitterrace_web::PAGE_CSP;
    format!(
        "/*\n  Content-Security-Policy: {page}\n  X-Content-Type-Options: nosniff\n  \
         X-Frame-Options: DENY\n  Referrer-Policy: strict-origin-when-cross-origin\n  \
         Strict-Transport-Security: max-age=63072000; includeSubDomains\n  \
         X-Robots-Tag: noindex\n\n\
         /r/:name/raw/*\n  Content-Security-Policy: sandbox; default-src 'none'; style-src 'unsafe-inline'\n\n\
         /static/*\n  Cache-Control: public, max-age=3600\n"
    )
}

/// The set of paths an export follows.
struct Allowed {
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
        Ok(Self { commits, tree })
    }

    fn contains(&self, link: &str) -> bool {
        if seeds().any(|s| s == link) || link.starts_with("/static/") {
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
                // A static host serves an exported file by its extension, without the
                // server's sandbox header: an SVG or HTML file from the repository would run
                // its scripts on the export's origin. Only inert media leaves as raw bytes;
                // its link 404s in the copy like any unexported page.
                if prefix == "/raw/HEAD/" && !gitterrace_web::plain_media(rel) {
                    return false;
                }
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

#[cfg(test)]
mod tests;
