//! Route handlers. Each resolves its revision first, then reads only from that commit id.

use std::fmt::Write as _;
use std::sync::Arc;

use askama::Template;
use axum::{
    Json, Router,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use gitterrace_git::RefKind;
use serde::Deserialize;

use crate::{
    app::{App, blocking},
    error::Result,
    fmt,
    live::Mode,
    markdown, views,
    views::{Chrome, CommitRow, Crumb, Entry, FileStatRow, RefRow},
};

const PAGE: usize = 50;

pub(crate) type Shared = State<Arc<App>>;

pub(crate) fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(overview))
        .route(
            "/log",
            get(|State(a): Shared| async move { Redirect::to(&format!("{}/log/HEAD", a.base)) }),
        )
        .route("/log/{rev}", get(log))
        .route("/commit/{id}", get(commit))
        .route("/diff/{rev}/{index}", get(views::diff_fragment))
        .route("/tree/{rev}", get(tree_root))
        .route("/tree/{rev}/", get(tree_root))
        .route("/tree/{rev}/{*path}", get(tree))
        .route("/blob/{rev}/{*path}", get(blob))
        .route("/blame/{rev}/{*path}", get(blame))
        .route("/hl/{rev}/{*path}", get(highlight_tail))
        .route("/refs", get(refs))
        .route("/search", get(crate::search::search))
        .route("/worktree", get(worktree))
        .route("/api/files/{rev}", get(api_files))
        .route("/api/refs", get(api_refs))
        .route("/api/last/{rev}", get(api_last_root))
        .route("/api/last/{rev}/{*path}", get(api_last))
        .route("/stats", get(crate::pages::activity))
        .route("/stats/files", get(moved("/stats/hotspots")))
        .route("/stats/types", get(crate::types::page))
        .route("/stats/languages", get(crate::languages::page))
        .route("/stats/frequency", get(crate::insights::code_frequency))
        .route("/stats/pulse", get(moved("/stats")))
        .route("/stats/hotspots", get(crate::pages::hotspots))
        .route("/stats/ownership", get(crate::pages::ownership))
        .route("/stats/patterns", get(crate::pages::patterns))
        .route("/compare", get(crate::pages::compare))
        .route("/history/{*path}", get(crate::pages::history))
        .route("/stats/contributors", get(crate::insights::contributors))
        .route("/raw/{rev}/{*path}", get(raw))
        .route("/api/progress", get(progress))
        .route("/worktrees", get(crate::gitstate::worktrees))
        .route("/stashes", get(crate::gitstate::stashes))
        .route("/tag/{name}", get(crate::gitstate::tag))
        .route("/events", get(crate::live::events))
        .layer(axum::middleware::from_fn(selected_rev))
        .merge(assets())
        .with_state(app)
}

/// Makes `?rev=` the selected ref for everything the request renders.
async fn selected_rev(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let rev = fmt::rev_param(req.uri().query());
    fmt::REQUEST_REV.scope(rev, next.run(req)).await
}

/// Embedded stylesheets, scripts and fonts; stateless, so the multi-repo index serves them too.
pub(crate) fn assets<S: Clone + Send + Sync + 'static>() -> Router<S> {
    const JS: &str = "text/javascript; charset=utf-8";
    const CSS: &str = "text/css; charset=utf-8";
    let table: [(&str, &str, &'static str); 11] = [
        ("loc.js", JS, include_str!("../static/loc.js")),
        ("tokens.css", CSS, include_str!("../static/tokens.css")),
        ("app.css", CSS, include_str!("../static/app.css")),
        ("tip.js", JS, include_str!("../static/tip.js")),
        ("app.js", JS, include_str!("../static/app.js")),
        ("markdown.css", CSS, include_str!("../static/markdown.css")),
        ("markdown.js", JS, include_str!("../static/markdown.js")),
        ("nav.js", JS, include_str!("../static/nav.js")),
        ("actions.js", JS, include_str!("../static/actions.js")),
        ("send.js", JS, include_str!("../static/send.js")),
        ("live.js", JS, include_str!("../static/live.js")),
    ];
    table
        .into_iter()
        .fold(Router::new(), |r, (name, kind, body)| {
            r.route(&format!("/static/{name}"), static_asset(kind, body))
        })
        .route("/static/vendor/mermaid-11.17.2.min.js", get(mermaid_js))
        .route("/static/fonts/{name}", get(font))
}

/// Vendored OFL variable fonts, Latin subsets: Fraunces (display), Bricolage Grotesque (body)
/// and `JetBrains` Mono (code).
const FONTS: [(&str, &[u8]); 3] = [
    (
        "fraunces.woff2",
        include_bytes!(
            "../../../theme/schwarzwald/fonts/fraunces/fraunces-latin-wght-normal.woff2"
        ),
    ),
    (
        "bricolage-grotesque.woff2",
        include_bytes!(
            "../../../theme/schwarzwald/fonts/bricolage-grotesque/bricolage-grotesque-latin-wght-normal.woff2"
        ),
    ),
    (
        "jetbrains-mono.woff2",
        include_bytes!(
            "../../../theme/schwarzwald/fonts/jetbrains-mono/jetbrains-mono-latin-wght-normal.woff2"
        ),
    ),
];

async fn font(Path(name): Path<String>) -> Response {
    let Some((_, body)) = FONTS.iter().find(|(n, _)| *n == name) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("font/woff2")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        *body,
    )
        .into_response()
}

static MERMAID: &[u8] = include_bytes!("../static/vendor/mermaid.min.js");

async fn mermaid_js() -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/javascript; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        MERMAID,
    )
        .into_response()
}

/// Largest media file the blob page reads, for its pixel size; bigger ones show no facts.
pub(crate) const MAX_PREVIEW_BYTES: u64 = 16 << 20;

/// Whether a blob of `size` bytes is too big for `/raw` under `limits.raw_mb`.
pub(crate) const fn raw_refused(size: u64, raw_mb: u64) -> bool {
    size > raw_mb << 20
}

/// Whether an `If-None-Match` header lists `tag` (or `*`).
fn tag_matches(headers: &axum::http::HeaderMap, tag: &str) -> bool {
    headers.get(header::IF_NONE_MATCH).is_some_and(|v| {
        v.as_bytes()
            .split(|b| *b == b',')
            .any(|t| t.trim_ascii() == tag.as_bytes() || t.trim_ascii() == b"*")
    })
}

/// What `/raw` found: the blob is too large, the client's copy is current, or the bytes.
enum Raw {
    TooLarge,
    Fresh(String),
    Bytes(Vec<u8>, String),
}

/// Serves blob bytes for relative images. Content is untrusted, so scripts are disabled.
/// The `ETag` is the blob id, so a repeat visit costs a 304 and no content read.
async fn raw(
    State(app): Shared,
    headers: axum::http::HeaderMap,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Response> {
    let name = path.trim_matches('/').to_owned();
    let limit = app.cfg().limits.raw_mb;
    let loaded = blocking(&app, move |a| {
        // One resolve and one tree walk; the header size comes before any content read.
        let blob = a.repo.raw_blob(&rev, &name, |id, size| {
            raw_refused(size, limit) || tag_matches(&headers, &format!("\"{id}\""))
        })?;
        let tag = format!("\"{}\"", blob.id);
        Ok(match blob.bytes {
            Some(bytes) => Raw::Bytes(bytes, tag),
            None if raw_refused(blob.size, limit) => Raw::TooLarge,
            None => Raw::Fresh(tag),
        })
    })
    .await?;
    let (bytes, tag) = match loaded {
        Raw::TooLarge => {
            return Ok((StatusCode::PAYLOAD_TOO_LARGE, "too large to serve\n").into_response());
        }
        Raw::Fresh(tag) => {
            let tag =
                HeaderValue::from_str(&tag).unwrap_or_else(|_| HeaderValue::from_static("\"0\""));
            return Ok((
                StatusCode::NOT_MODIFIED,
                [
                    (header::ETAG, tag),
                    (
                        header::CACHE_CONTROL,
                        HeaderValue::from_static("private, max-age=300"),
                    ),
                ],
            )
                .into_response());
        }
        Raw::Bytes(bytes, tag) => (bytes, tag),
    };
    // PDFs never open in the browser's viewer on this origin.
    let disposition = if crate::media::kind(&path) == Some(crate::media::Kind::Pdf) {
        "attachment"
    } else {
        "inline"
    };
    let (kind, plain) = (
        crate::media::content_type(&path),
        crate::media::plain_media(&path),
    );
    // The sandbox keeps SVG and text scriptless. It also stops the browser's PDF viewer, so
    // formats that run no author code are served without it.
    let csp = if plain {
        "default-src 'none'; style-src 'unsafe-inline'"
    } else {
        "sandbox; default-src 'none'; style-src 'unsafe-inline'"
    };
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(kind)),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(csp),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static(disposition),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=300"),
            ),
            (
                header::ETAG,
                HeaderValue::from_str(&tag).unwrap_or_else(|_| HeaderValue::from_static("\"0\"")),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// A strong `ETag` from the embedded bytes: stable for one build, changes with the content.
fn etag(body: &str) -> HeaderValue {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut h);
    HeaderValue::from_str(&format!("\"{:016x}-{:x}\"", h.finish(), body.len()))
        .unwrap_or_else(|_| HeaderValue::from_static("\"0\""))
}

/// Revalidating route for an embedded file: `no-cache` plus an `ETag`, so a repeat visit costs a 304.
fn static_asset<S: Clone + Send + Sync + 'static>(
    kind: &'static str,
    body: &'static str,
) -> axum::routing::MethodRouter<S> {
    let tag = etag(body);
    get(move |headers: axum::http::HeaderMap| {
        let tag = tag.clone();
        async move { asset(&headers, kind, body, tag) }
    })
}

fn asset(
    headers: &axum::http::HeaderMap,
    kind: &'static str,
    body: &'static str,
    tag: HeaderValue,
) -> Response {
    let fresh = headers.get(header::IF_NONE_MATCH).is_some_and(|v| {
        v.as_bytes()
            .split(|b| *b == b',')
            .any(|t| t.trim_ascii() == tag.as_bytes())
    });
    let meta = [
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        (header::ETAG, tag),
    ];
    if fresh {
        return (axum::http::StatusCode::NOT_MODIFIED, meta).into_response();
    }
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static(kind))],
        meta,
        body,
    )
        .into_response()
}

fn page<T: Template>(t: &T) -> Result<Html<String>> {
    Ok(Html(t.render()?))
}

struct Numbers {
    commits: String,
    commits_noun: &'static str,
    authors: String,
    authors_noun: &'static str,
    files: String,
    files_noun: &'static str,
    additions: String,
    deletions: String,
    lines_noun: &'static str,
}

/// `?doc=` picks the tab of the about card.
#[derive(Deserialize)]
struct DocQuery {
    doc: Option<String>,
}

#[derive(Template)]
#[template(path = "overview.html")]
struct OverviewPage {
    c: Chrome,
    totals: Option<Numbers>,
    about: Option<crate::about::About>,
    entries: Vec<views::Entry>,
    latest: Option<CommitRow>,
    insights_href: String,
}

async fn overview(State(app): Shared, Query(q): Query<DocQuery>) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let rev = fmt::selected();
        let head = a.repo.resolve(&rev)?;
        let now = fmt::now();
        let recent = a.repo.log(&head, None, 0, 1)?;
        let list = a.repo.tree(&head, "")?;
        let docs = crate::about::find(&list);
        let about = crate::about::card(a, &head, &rev, "", "/", &docs, q.doc.as_deref());
        let totals = a.records(&head)?.map(|r| {
            let t = crate::insights::insights(a, &r, a.defaults().into_options())
                .period_summary(gitterrace_index::Period::All)
                .current;
            Numbers {
                commits: fmt::thousands(t.commits),
                commits_noun: fmt::noun(t.commits, "commit", "commits"),
                authors: fmt::thousands(t.authors),
                authors_noun: fmt::noun(t.authors, "author", "authors"),
                files: fmt::thousands(t.files),
                files_noun: fmt::noun(t.files, "file touched", "files touched"),
                additions: fmt::thousands(t.additions),
                deletions: fmt::thousands(t.deletions),
                lines_noun: fmt::noun(t.additions + t.deletions, "line", "lines"),
            }
        });
        Ok(OverviewPage {
            c: Chrome {
                md: about.as_ref().is_some_and(|d| d.markdown),
                ..views::chrome(a, "files")
            }
            .live(Mode::NOTICE),
            totals,
            about,
            entries: views::entries(&rev, "", &list),
            latest: recent.first().map(|c| views::commit_row(c, now)),
            insights_href: fmt::href("/stats"),
        })
    })
    .await?;
    page(&p)
}

#[derive(Deserialize)]
struct LogQuery {
    path: Option<String>,
    page: Option<usize>,
    author: Option<String>,
}

pub(crate) struct DayGroup {
    pub(crate) label: String,
    pub(crate) rows: Vec<CommitRow>,
}

#[derive(Template)]
#[template(path = "log.html")]
struct LogPage {
    c: Chrome,
    rev: String,
    rev_enc: String,
    path: String,
    days: Vec<DayGroup>,
    empty: bool,
    newer: Option<String>,
    older: Option<String>,
    /// Removable chip for the active author filter: (name, href without it).
    author_chip: Option<(String, String)>,
}

/// Pages past this are clamped: the walk behind a page costs its offset.
const MAX_PAGE: usize = 1_000;
/// Commits per read of the unindexed author walk.
const CHUNK: usize = 500;
/// Commits the unindexed author walk reads before it gives up with what it has.
const AUTHOR_WALK: usize = 2_000;

/// Whether `who` (email or name) wrote the commit.
fn by(who: &str, author: &str, email: &str) -> bool {
    email.eq_ignore_ascii_case(who) || author == who
}

/// Ids of the commits by `who` touching `path`, newest first, from the index records
/// (oldest first), so no history is walked.
fn indexed_author_ids(
    recs: &[gitterrace_index::CommitRecord],
    who: &str,
    path: Option<&str>,
) -> Vec<String> {
    recs.iter()
        .rev()
        .filter(|r| by(who, &r.author, &r.email))
        .filter(|r| path.is_none_or(|p| r.files.iter().any(|f| crate::filter::under(&f.path, p))))
        .map(|r| r.id.clone())
        .collect()
}

/// One page of commits by `who`. With a ready index the records answer it. Without one the
/// log is walked in chunks (the walk cannot filter by author), up to `AUTHOR_WALK` commits.
fn author_page(
    a: &App,
    id: &str,
    path: Option<&str>,
    who: &str,
    pg: usize,
) -> Result<(Vec<gitterrace_git::CommitSummary>, bool)> {
    let from = pg * PAGE;
    if let Some(recs) = a.records(id)? {
        let ids = indexed_author_ids(&recs, who, path);
        let more = ids.len() > from + PAGE;
        let slice: Vec<String> = ids.into_iter().skip(from).take(PAGE).collect();
        return Ok((a.repo.summaries(&slice)?, more));
    }
    let want = from + PAGE + 1;
    let mut hits = Vec::new();
    for skip in (0..AUTHOR_WALK).step_by(CHUNK) {
        let chunk = a.repo.log(id, path, skip, CHUNK)?;
        let n = chunk.len();
        hits.extend(
            chunk
                .into_iter()
                .filter(|c| by(who, &c.author.name, &c.author.email)),
        );
        if hits.len() >= want || n < CHUNK {
            break;
        }
    }
    let mut page: Vec<_> = hits.into_iter().skip(from).collect();
    let more = page.len() > PAGE;
    page.truncate(PAGE);
    Ok((page, more))
}

async fn log(
    State(app): Shared,
    Path(rev): Path<String>,
    Query(q): Query<LogQuery>,
) -> Result<Html<String>> {
    let path = q.path.unwrap_or_default().trim_matches('/').to_owned();
    let pg = q.page.unwrap_or(0).min(MAX_PAGE);
    let author = q.author;
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let filter = (!path.is_empty()).then_some(path.as_str());
        // One extra row tells us whether an older page exists.
        let who = author.as_deref().filter(|s| !s.is_empty());
        let (mut list, more) = if let Some(w) = who {
            author_page(a, &id, filter, w, pg)?
        } else {
            let mut list = a.repo.log(&id, filter, pg.saturating_mul(PAGE), PAGE + 1)?;
            let more = list.len() > PAGE;
            list.truncate(PAGE);
            (list, more)
        };
        let author_chip = who.map(|w| {
            let name = list
                .first()
                .map_or_else(|| w.to_owned(), |c| c.author.name.clone());
            let mut back = fmt::href(&format!("/log/{}", fmt::enc_seg(&rev)));
            if !path.is_empty() {
                let _ = write!(back, "?path={}", fmt::enc_path(&path));
            }
            (name, back)
        });
        list.shrink_to_fit();
        let now = fmt::now();
        let rev_enc = fmt::enc_seg(&rev);
        let link = |n: usize| {
            let mut q = format!("/log/{rev_enc}?page={n}");
            if !path.is_empty() {
                let _ = write!(q, "&path={}", fmt::enc_path(&path));
            }
            if let Some(w) = who {
                let _ = write!(q, "&author={}", fmt::enc_seg(w));
            }
            fmt::href(&q)
        };
        Ok(LogPage {
            c: views::chrome_at(a, "log", &rev).live(Mode::NOTICE),
            newer: (pg > 0).then(|| link(pg - 1)),
            older: more.then(|| link(pg + 1)),
            author_chip,
            empty: list.is_empty(),
            days: views::day_groups(&list, now),
            rev,
            rev_enc,
            path,
        })
    })
    .await?;
    page(&p)
}

#[derive(Template)]
#[template(path = "commit.html")]
struct CommitPage {
    c: Chrome,
    id: String,
    sha: String,
    subject: String,
    body: String,
    author: String,
    author_href: String,
    date: String,
    parents: Vec<String>,
    files_summary: String,
    files: Vec<FileStatRow>,
    /// `(label, url)`: "Find this commit in ..." links from the `lookup` of each Send to target.
    lookups: Vec<(String, String)>,
    diff: String,
}

async fn commit(State(app): Shared, Path(id): Path<String>) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&id)?;
        let d = a.repo.commit(&id)?;
        Ok(CommitPage {
            c: Chrome {
                commit: id.clone(),
                rev: id.clone(),
                rev_enc: id.clone(),
                ..views::chrome(a, "log")
            }
            .pinned(),
            sha: fmt::sha_link(&id),
            subject: d.summary.subject.clone(),
            body: d.body.trim().to_owned(),
            author: d.summary.author.name.clone(),
            author_href: views::author_href(&d.summary.author.email),
            date: fmt::date(d.summary.author.time, d.summary.author.offset),
            parents: d.summary.parents.iter().map(|p| fmt::sha_link(p)).collect(),
            files_summary: fmt::count(d.files.len(), "changed file", "changed files"),
            files: views::file_stats(&d),
            lookups: a.lookups(&id),
            diff: views::diff_html(
                a.cfg().limits.diff_fold_lines,
                &id,
                d.summary.parents.first().map(String::as_str),
                &d.files,
            ),
            id,
        })
    })
    .await?;
    page(&p)
}

#[derive(Template)]
#[template(path = "tree.html")]
struct TreePage {
    c: Chrome,
    title: String,
    rev_enc: String,
    path_enc: String,
    crumbs: Vec<Crumb>,
    up: Option<String>,
    entries: Vec<Entry>,
    about: Option<crate::about::About>,
}

async fn tree_root(
    state: Shared,
    Path(rev): Path<String>,
    q: Query<DocQuery>,
) -> Result<Html<String>> {
    tree(state, Path((rev, String::new())), q).await
}

async fn tree(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
    Query(q): Query<DocQuery>,
) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let list = a.repo.tree(&id, &path)?;
        let rev_enc = fmt::enc_seg(&rev);
        let up = path
            .rsplit_once('/')
            .map(|(parent, _)| fmt::href(&format!("/tree/{rev_enc}/{}", fmt::enc_path(parent))));
        let up = match (up, path.is_empty()) {
            (Some(u), _) => Some(u),
            (None, false) => Some(fmt::href(&format!("/tree/{rev_enc}"))),
            (None, true) => None,
        };
        let docs = crate::about::find(&list);
        let page = format!("/tree/{rev_enc}/{}", fmt::enc_path(&path));
        let about = crate::about::card(
            a,
            &id,
            &rev,
            &path,
            page.trim_end_matches('/'),
            &docs,
            q.doc.as_deref(),
        );
        Ok(TreePage {
            c: Chrome {
                md: about.as_ref().is_some_and(|d| d.markdown),
                ..views::chrome_at(a, "files", &rev)
            },
            about,
            title: if path.is_empty() {
                rev.clone()
            } else {
                path.clone()
            },
            crumbs: views::crumbs(&rev, &path, false),
            entries: views::entries(&rev, &path, &list),
            path_enc: fmt::enc_path(&path),
            rev_enc,
            up,
        })
    })
    .await?;
    page(&p)
}

#[derive(Template)]
#[template(path = "blob.html")]
struct BlobPage {
    c: Chrome,
    title: String,
    crumbs: Vec<Crumb>,
    size: String,
    blame_href: String,
    log_href: String,
    html: Option<String>,
    cut: bool,
    media: Option<String>,
    too_large: bool,
    raw_href: String,
    rendered: Option<String>,
}

fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".md", ".markdown", ".mdown"]
        .iter()
        .any(|e| lower.ends_with(e))
}

/// The highlighted chunks a big blob's first paint left plain; `nav.js` swaps them in.
async fn highlight_tail(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Html<String>> {
    let html = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let text = a.repo.blob(&id, &path)?.text.unwrap_or_default();
        Ok(views::code_tail_html(a.code_cache_bytes(), &path, &text))
    })
    .await?;
    Ok(Html(html))
}

async fn blob(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let b = a.repo.blob(&id, &path)?;
        let media = match crate::media::kind(&path) {
            Some(k) => {
                // One read, and only for files small enough to want facts about.
                let bytes = if b.size <= MAX_PREVIEW_BYTES {
                    a.repo.blob_bytes(&id, &path)?
                } else {
                    Vec::new()
                };
                let src = (k == crate::media::Kind::Svg)
                    .then(|| std::str::from_utf8(&bytes).ok())
                    .flatten()
                    .filter(|t| t.len() as u64 <= a.repo.text_limit());
                Some(crate::media::blob_html(k, b.size, &rev, &path, &bytes, src))
            }
            None => None,
        };
        let too_large = b.size > a.repo.text_limit();
        let text = b.text.as_deref().filter(|_| !too_large);
        let (html, cut) = text.map_or((None, false), |t| {
            let (h, cut) = views::code_html_first(a.code_cache_bytes(), &path, t);
            (Some(h), cut)
        });
        let rendered = text.filter(|_| is_markdown(&path)).map(|t| {
            let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
            markdown::render_with(t, markdown::Base::new(&rev, dir))
        });
        let (r, pe) = (fmt::enc_seg(&rev), fmt::enc_path(&path));
        Ok(BlobPage {
            c: Chrome {
                md: rendered.is_some(),
                ..views::chrome_at(a, "files", &rev)
            },
            rendered,
            crumbs: views::crumbs(&rev, &path, true),
            size: fmt::size(b.size),
            blame_href: fmt::href(&format!("/blame/{r}/{pe}")),
            log_href: fmt::href(&format!("/log/{r}?path={pe}")),
            title: path,
            html,
            cut,
            media,
            too_large,
            raw_href: fmt::href(&format!("/raw/{r}/{pe}")),
        })
    })
    .await?;
    page(&p)
}

#[derive(Template)]
#[template(path = "blame.html")]
struct BlamePage {
    c: Chrome,
    title: String,
    crumbs: Vec<Crumb>,
    blob_href: String,
    html: String,
}

async fn blame(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let lines = a.repo.blame(&id, &path)?;
        // Blame is empty for binary and oversized files; say which, from the header size.
        let too_large =
            lines.is_empty() && a.repo.blob(&id, &path)?.size > a.repo.text_limit();
        let raw = fmt::href(&format!(
            "/raw/{}/{}",
            fmt::enc_seg(&rev),
            fmt::enc_path(&path)
        ));
        Ok(BlamePage {
            c: views::chrome_at(a, "files", &rev),
            crumbs: views::crumbs(&rev, &path, true),
            blob_href: fmt::href(&format!(
                "/blob/{}/{}",
                fmt::enc_seg(&rev),
                fmt::enc_path(&path)
            )),
            html: if too_large {
                format!(
                    "<p class=\"muted pad\">File too large to blame. <a href=\"{}\">View raw</a>.</p>",
                    fmt::esc(&raw)
                )
            } else {
                views::blame_html(&path, &lines, fmt::now())
            },
            title: path,
        })
    })
    .await?;
    page(&p)
}

struct TagRow {
    name: String,
    href: String,
    subject: String,
    commit: String,
    when: String,
}

#[derive(Template)]
#[template(path = "refs.html")]
struct RefsPage {
    c: Chrome,
    branches: Vec<crate::gitstate::BranchRow>,
    tags: Vec<TagRow>,
    remotes: Vec<RefRow>,
    reftabs: Vec<views::RefTab>,
}

async fn refs(State(app): Shared) -> Result<Html<String>> {
    let p = blocking(&app, |a| {
        let all = a.repo.refs()?;
        let now = fmt::now();
        let tags = all
            .iter()
            .filter(|r| r.kind == RefKind::Tag)
            .map(|r| TagRow {
                href: fmt::href(&format!("/tag/{}", fmt::enc_seg(&r.name))),
                subject: r
                    .annotation
                    .as_ref()
                    .map(|n| n.subject.clone())
                    .unwrap_or_default(),
                commit: fmt::sha_link(&r.target),
                when: fmt::ago(r.time, now),
                name: r.name.clone(),
            })
            .collect();
        Ok(RefsPage {
            c: views::chrome(a, "refs").live(Mode::REFS),
            branches: crate::gitstate::branch_rows(a)?,
            tags,
            remotes: views::ref_rows(&all, RefKind::Remote, now),
            reftabs: views::ref_tabs(a, "refs")?,
        })
    })
    .await?;
    page(&p)
}

/// A permanent redirect from a removed insights page to the one that replaced it, the query
/// (filters, period, options) kept so old links land on the same selection.
fn moved(
    to: &'static str,
) -> impl Fn(State<Arc<App>>, RawQuery) -> std::future::Ready<Response> + Clone + Send + 'static {
    move |State(a), RawQuery(q)| {
        let q = q.map(|q| format!("?{q}")).unwrap_or_default();
        let at = format!("{}{to}{q}", a.base);
        std::future::ready(
            (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, at)]).into_response(),
        )
    }
}

#[derive(Template)]
#[template(path = "pending.html")]
struct PendingPage {
    c: Chrome,
    tabs: views::Tabs,
    /// Set when a newer build owns the cache: this one leaves it alone instead of indexing.
    newer: Option<String>,
}

/// Runs `build` over the cached records, or yields the progress page while none exist yet.
pub(crate) async fn with_records<T: Template + Send + 'static>(
    app: &Arc<App>,
    tab: &'static str,
    build: impl FnOnce(&App, &[gitterrace_index::CommitRecord], Chrome) -> T + Send + 'static,
) -> Result<Html<String>> {
    let html = blocking(app, move |a| {
        let head = a.repo.resolve(&fmt::selected())?;
        let c = views::chrome(a, "stats").live(Mode::NOTICE);
        match a.records(&head)? {
            Some(r) => Ok(build(a, &r, c).render()?),
            None => Ok(PendingPage {
                c,
                tabs: views::stats_tabs(tab),
                newer: a.index.newer_format().map(|v| {
                    format!(
                        "The history cache was written by a newer gitterrace (format {v}, this \
                         build reads {}). It is left untouched: run the newer build, or delete \
                         {} to rebuild it here.",
                        gitterrace_index::FORMAT_VERSION,
                        a.index.dir().display()
                    )
                }),
            }
            .render()?),
        }
    })
    .await?;
    Ok(Html(html))
}

#[derive(serde::Serialize)]
struct ProgressBody {
    done: usize,
    total: usize,
    ready: bool,
}

async fn progress(State(app): Shared) -> Json<ProgressBody> {
    let p = app.progress();
    Json(ProgressBody {
        done: p.done,
        total: p.total,
        ready: app.ready(),
    })
}

#[derive(Template)]
#[template(path = "worktree.html")]
struct WorktreePage {
    c: Chrome,
    files_summary: String,
    files: Vec<FileStatRow>,
    diff: String,
    clean: bool,
    adds: String,
    dels: String,
}

async fn worktree(State(app): Shared) -> Result<Html<String>> {
    let p = blocking(&app, |a| {
        let files = gitterrace_git::worktree_diff(&a.repo)?;
        let (adds, dels) = files
            .iter()
            .fold((0, 0), |(x, y), f| (x + f.additions, y + f.deletions));
        let stats: Vec<FileStatRow> = files
            .iter()
            .enumerate()
            .map(|(i, f)| FileStatRow {
                anchor: format!("f{i}"),
                label: views::file_label(f),
                additions: fmt::thousands(f.additions),
                deletions: fmt::thousands(f.deletions),
            })
            .collect();
        Ok(WorktreePage {
            c: views::chrome(a, "files").live(Mode::WORKTREE),
            clean: files.is_empty(),
            diff: views::diff_html_with(a.cfg().limits.diff_fold_lines, None, None, &files),
            adds: fmt::thousands(adds),
            dels: fmt::thousands(dels),
            files_summary: fmt::count(stats.len(), "changed file", "changed files"),
            files: stats,
        })
    })
    .await?;
    page(&p)
}

fn json<T: serde::Serialize>(v: &T) -> Response {
    match serde_json::to_vec(v) {
        Ok(b) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            ],
            b,
        )
            .into_response(),
        Err(e) => crate::error::Error::Index(std::io::Error::other(e)).into_response(),
    }
}

/// All file paths at a revision, for the client-side finder.
async fn api_files(State(app): Shared, Path(rev): Path<String>) -> Result<Response> {
    let paths = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        a.file_paths(&id)
    })
    .await?;
    Ok(json(&*paths))
}

#[derive(serde::Serialize)]
struct RefBody {
    name: String,
    kind: &'static str,
    when: String,
}

/// Branches and tags for the switcher. Remotes are left out to keep the list short.
async fn api_refs(State(app): Shared) -> Result<Response> {
    let list = blocking(&app, |a| {
        let now = fmt::now();
        Ok(a.repo
            .refs()?
            .into_iter()
            .filter(|r| r.kind != RefKind::Remote)
            .map(|r| RefBody {
                kind: if r.kind == RefKind::Tag {
                    "tag"
                } else {
                    "branch"
                },
                when: fmt::ago(r.time, now),
                name: r.name,
            })
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(json(&list))
}

#[derive(serde::Serialize)]
struct LastBody {
    name: String,
    id: String,
    subject: String,
    when: String,
}

async fn api_last_root(state: Shared, Path(rev): Path<String>) -> Result<Response> {
    api_last(state, Path((rev, String::new()))).await
}

/// Last commit per entry of a directory, filled into the tree after first paint.
async fn api_last(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Response> {
    let list = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let now = fmt::now();
        let found = a.last_commits(&id, path.trim_matches('/'))?;
        Ok(found
            .iter()
            .map(|c| LastBody {
                name: c.name.clone(),
                id: c.id.clone(),
                subject: c.subject.clone(),
                when: fmt::ago(c.time, now),
            })
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(json(&list))
}

#[cfg(test)]
mod tests {
    use gitterrace_index::{CommitRecord, FileStat};

    use crate::routes::indexed_author_ids;

    fn rec(id: &str, author: &str, email: &str, path: &str) -> CommitRecord {
        CommitRecord {
            id: id.into(),
            author: author.into(),
            email: email.into(),
            time: 0,
            offset: 0,
            merge: false,
            parents: Vec::new(),
            kind: None,
            scope: None,
            files: vec![FileStat {
                path: path.into(),
                ..FileStat::default()
            }],
        }
    }

    #[test]
    fn author_log_comes_from_the_records_newest_first() {
        let recs = [
            rec("a", "Ann", "ann@x.y", "src/a.rs"),
            rec("b", "Bob", "bob@x.y", "src/b.rs"),
            rec("c", "Ann", "ANN@x.y", "docs/c.md"),
        ];
        assert_eq!(indexed_author_ids(&recs, "ann@x.y", None), ["c", "a"]);
        assert_eq!(indexed_author_ids(&recs, "Ann", Some("src")), ["a"]);
        assert!(indexed_author_ids(&recs, "nobody", None).is_empty());
    }
}
