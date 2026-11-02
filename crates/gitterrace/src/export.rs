//! `export`: render the site to static files by driving the router in-process.
//!
//! A full crawl of a real repository visits every commit and every blob, which
//! is tens of thousands of pages. The export is therefore scoped: the
//! overview, the insights pages, branches and tags, the newest N commits with
//! their log pages, and the file tree plus files at HEAD up to a size cap.
//! Links to anything outside that set are left pointing at their original path
//! and 404 in the exported copy, the same as a link to a page that was
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
use gitterrace_git::{percent_decode, percent_encode_path};
use tower::ServiceExt;

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

    let mut queue: VecDeque<String> = VecDeque::new();
    if repos.len() > 1 {
        queue.push_back("/".to_owned());
    }
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

/// One repository in an export: where its pages live and which of them are in scope.
struct Site {
    /// `""` for a single repository, `/r/<name>` among several.
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

/// Where each repository's pages live, in `repos` order, from the server itself: one
/// repository serves at `/`, several under the names `/api/repos` lists. Asking the router
/// keeps naming (and its collision rule) in one place.
async fn sites(router: &Router, repos: &[PathBuf]) -> anyhow::Result<Vec<String>> {
    if repos.len() < 2 {
        return Ok(vec![String::new()]);
    }
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
    eprintln!(
        "building the cache for {} (first run reads every commit once)...",
        if prefix.is_empty() { "/" } else { prefix }
    );
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
         /raw/*\n  Content-Security-Policy: sandbox; default-src 'none'; style-src 'unsafe-inline'\n\n\
         /r/*/raw/*\n  Content-Security-Policy: sandbox; default-src 'none'; style-src 'unsafe-inline'\n\n\
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

/// Inverse of the web's `fmt::enc`: `%XX` back to bytes, then UTF-8. Links are
/// matched and written by their real name, so `a b.md` exports as `a b.md`, not
/// `a%20b.md`, which a static server would decode and then miss.
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
fn mark_snapshot(html: &[u8], label: &str, project: Option<&Project>) -> Vec<u8> {
    let page = String::from_utf8_lossy(html);
    // Anchor after `</head>`: a `<body` inside a title, a meta value or a script string in the
    // head is text, not the element.
    let from = page.find("</head>").unwrap_or(0);
    let Some(open) = ["<body ", "<body>"]
        .iter()
        .filter_map(|tag| page[from..].find(tag))
        .min()
        .map(|i| from + i)
    else {
        return html.to_vec();
    };
    let Some(end) = page[open..].find('>').map(|i| open + i + 1) else {
        return html.to_vec();
    };
    let what = match project {
        Some(p) => format!(
            "Static snapshot of <a href=\"{}\">{}</a> at <b>{}</b>, shown in gitterrace.",
            gitterrace_web::escape_html(&p.url),
            gitterrace_web::escape_html(&p.name),
            gitterrace_web::escape_html(label)
        ),
        // The index of several repositories names no revision of its own.
        None if label.is_empty() => "Static snapshot.".to_owned(),
        None => format!(
            "Static snapshot at <b>{}</b>.",
            gitterrace_web::escape_html(label)
        ),
    };
    let banner = format!(
        "<link rel=\"stylesheet\" href=\"/snapshot.css\">\
         <div class=\"snapshot\" role=\"note\"><div class=\"wrap\">{what} \
         Made with gitterrace {version}. \
         Search, live updates and actions need the real thing. \
         <a href=\"https://gitterrace.dev\">Install gitterrace</a></div></div>",
        version = env!("GITTERRACE_VERSION")
    );
    // The same version as a generator tag, so a copy of a single page still says what made it.
    let generator = format!(
        "<meta name=\"generator\" content=\"gitterrace {}\">",
        env!("GITTERRACE_VERSION")
    );
    let head_end = page.find("</head>").unwrap_or(0);
    let mut out = String::with_capacity(page.len() + banner.len() + generator.len() + 12);
    // `data-static` tells the page scripts there is no server: they skip live updates and
    // the actions poll, and the snapshot CSS hides what needs one.
    out.push_str(&page[..head_end]);
    out.push_str(&generator);
    out.push_str(&page[head_end..end - 1]);
    out.push_str(" data-static>");
    out.push_str(&banner);
    out.push_str(&page[end..]);
    out.into_bytes()
}

/// File the page lands in: HTML pages become `<path>/index.html` so their links
/// resolve when the folder is served by any static server. `None` for a path with a segment
/// that is not a plain name (`.`, `..`, a separator or drive colon on some platform, NUL):
/// such a page is skipped rather than written somewhere it was not named.
fn target(out: &Path, path: &str, content_type: &str) -> Option<PathBuf> {
    let mut p = out.to_path_buf();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        let hostile = seg == "." || seg == ".." || seg.contains(['\\', ':', '\0']);
        if hostile {
            return None;
        }
        p.push(seg);
    }
    if content_type.starts_with("text/html") {
        p.push("index.html");
    }
    Some(p)
}

#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// Writes one page; `false` when its path is refused by `target` and the page is skipped.
fn write_page(out: &Path, path: &str, content_type: &str, body: &[u8]) -> anyhow::Result<bool> {
    let Some(file) = target(out, path, content_type) else {
        eprintln!("skipping {path:?}: not a plain file name");
        return Ok(false);
    };
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
    write_leaf(&file, body)?;
    Ok(true)
}

/// Through the shared no-follow temp-and-rename writer: a link at the leaf is refused, never
/// written through, and a crash leaves the old page whole.
fn write_leaf(file: &Path, body: &[u8]) -> anyhow::Result<()> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name().and_then(|n| n.to_str())) else {
        bail!("refusing to write {}: no file name", file.display());
    };
    gitterrace_git::write_atomic(dir, name, body, 0o644)
        .with_context(|| format!("refusing to write {}", file.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[cfg(unix)]
    use crate::export::write_page;
    use crate::export::{links, mark_snapshot, target};

    #[test]
    fn headers_file_carries_the_page_policy() {
        let h = super::headers_file();
        assert!(h.contains(gitterrace_web::PAGE_CSP));
        assert!(!h.contains("script-src 'self' 'unsafe-inline'"));
    }

    #[test]
    fn encoded_names_round_trip_to_real_file_names() {
        let html = r#"<a href="/blob/HEAD/docs/a%20b%23c.md">x</a><a href="/blob/HEAD/%C3%BCber.md">y</a>"#;
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
        let err = crate::export::export(&repos, &out, &scope(false))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not empty"), "{err}");
        assert_eq!(std::fs::read_dir(&out).expect("read").count(), 1);
        crate::export::export(&repos, &out, &scope(true))
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
        crate::export::export(&repos, &out, &scope(false))
            .await
            .expect("first");
        std::fs::create_dir_all(out.join("old/page")).expect("mkdir");
        std::fs::write(out.join("old/page/index.html"), "stale").expect("write");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).expect("mkdir");
        std::fs::write(outside.join("precious"), "keep").expect("write");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, out.join("link")).expect("symlink");
        crate::export::export(&repos, &out, &scope(false))
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
        crate::export::export(&[repo.path().to_path_buf()], &out, &scope(false))
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
        };
        let err = crate::export::export(&[repo.path().to_path_buf()], repo.path(), &scope)
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
        };
        crate::export::export(&[repo.path().to_path_buf()], &out, &scope)
            .await
            .expect("export");
        for page in gitterrace_web::WARM_PAGES {
            let file = target(&out, page, "text/html").expect("plain path");
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
        };
        crate::export::export(
            &[a.path().to_path_buf(), b.path().to_path_buf()],
            &out,
            &scope,
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
                let file =
                    target(&out, &format!("/r/{name}{page}"), "text/html").expect("plain path");
                assert!(file.is_file(), "/r/{name}{page} was not exported");
            }
        }
        // A repository's pages link only into itself, the index, or the shared assets.
        let alpha =
            std::fs::read_to_string(target(&out, "/r/alpha/", "text/html").expect("plain path"))
                .expect("alpha");
        for link in links(&alpha) {
            assert!(
                link.starts_with("/r/alpha")
                    || link == "/"
                    || link == "/snapshot.css"
                    || link.starts_with("/static/"),
                "alpha links outside itself: {link}"
            );
        }
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
        };
        crate::export::export(&[fx.path().to_path_buf()], &out, &scope)
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
            out.join("raw/HEAD/a.png").is_file(),
            "inert media must still export; found {raw:?}"
        );
    }
}
