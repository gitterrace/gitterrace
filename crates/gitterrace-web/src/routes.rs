//! Route handlers. Each resolves its revision first, then reads only from that commit id.

use std::sync::Arc;

use askama::Template;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use gitterrace_git::RefKind;
use gitterrace_index::{Granularity, Label};

use crate::{
    app::{App, blocking},
    charts,
    error::Result,
    fmt, markdown, views,
    views::{Chrome, CommitRow, Crumb, Entry, FileStatRow, Nav, RefRow, Row},
};

const PAGE: usize = 50;
const HOT_DAYS: i64 = 90;
const HOT_LIMIT: usize = 25;
const TABLE_LIMIT: usize = 30;
const README_NAMES: [&str; 4] = ["README.md", "README.markdown", "Readme.md", "readme.md"];

type Shared = State<Arc<App>>;

pub(crate) fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(overview))
        .route("/log", get(|| async { Redirect::to("/log/HEAD") }))
        .route("/log/{rev}", get(log))
        .route("/commit/{id}", get(commit))
        .route("/tree/{rev}", get(tree_root))
        .route("/tree/{rev}/", get(tree_root))
        .route("/tree/{rev}/{*path}", get(tree))
        .route("/blob/{rev}/{*path}", get(blob))
        .route("/blame/{rev}/{*path}", get(blame))
        .route("/refs", get(refs))
        .route("/search", get(crate::search::search))
        .route("/worktree", get(worktree))
        .route("/api/files/{rev}", get(api_files))
        .route("/api/refs", get(api_refs))
        .route("/api/last/{rev}", get(api_last_root))
        .route("/api/last/{rev}/{*path}", get(api_last))
        .route("/stats", get(stats))
        .route("/stats/files", get(stats_files))
        .route("/stats/types", get(stats_types))
        .route("/api/progress", get(progress))
        .route(
            "/static/tokens.css",
            get(|| async {
                asset(
                    "text/css; charset=utf-8",
                    include_str!("../static/tokens.css"),
                )
            }),
        )
        .route(
            "/static/app.css",
            get(|| async { asset("text/css; charset=utf-8", include_str!("../static/app.css")) }),
        )
        .route(
            "/static/app.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../static/app.js"),
                )
            }),
        )
        .route(
            "/static/markdown.css",
            get(|| async {
                asset(
                    "text/css; charset=utf-8",
                    include_str!("../static/markdown.css"),
                )
            }),
        )
        .route(
            "/static/markdown.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../static/markdown.js"),
                )
            }),
        )
        .route(
            "/static/nav.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../static/nav.js"),
                )
            }),
        )
        .route("/static/vendor/mermaid-11.17.2.min.js", get(mermaid_js))
        .route("/static/fonts/{name}", get(font))
        .route("/raw/{rev}/{*path}", get(raw))
        .with_state(app)
}

/// Candidate-look fonts, OFL, Latin subsets.
const FONTS: [(&str, &[u8]); 5] = [
    (
        "fraunces-600.woff2",
        include_bytes!("../static/fonts/fraunces-600.woff2"),
    ),
    (
        "bricolage-400.woff2",
        include_bytes!("../static/fonts/bricolage-400.woff2"),
    ),
    (
        "bricolage-600.woff2",
        include_bytes!("../static/fonts/bricolage-600.woff2"),
    ),
    (
        "jbmono-400.woff2",
        include_bytes!("../static/fonts/jbmono-400.woff2"),
    ),
    (
        "jbmono-600.woff2",
        include_bytes!("../static/fonts/jbmono-600.woff2"),
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

fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "text/plain; charset=utf-8",
    }
}

/// Serves blob bytes for relative images. Content is untrusted, so scripts are disabled.
async fn raw(State(app): Shared, Path((rev, path)): Path<(String, String)>) -> Result<Response> {
    let (bytes, kind) = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let bytes = a.repo.blob_bytes(&id, &path)?;
        Ok((bytes, content_type(&path)))
    })
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(kind)),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("sandbox; default-src 'none'; style-src 'unsafe-inline'"),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=300"),
            ),
        ],
        bytes,
    )
        .into_response())
}

fn asset(kind: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(kind)),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
        .into_response()
}

fn page<T: Template>(t: &T) -> Result<Html<String>> {
    Ok(Html(t.render()?))
}

struct Numbers {
    commits: String,
    authors: String,
    files: String,
    additions: String,
    deletions: String,
}

#[derive(Template)]
#[template(path = "overview.html")]
struct OverviewPage {
    c: Chrome,
    totals: Option<Numbers>,
    readme: Option<String>,
    entries: Vec<views::Entry>,
    latest: Option<CommitRow>,
}

async fn overview(State(app): Shared) -> Result<Html<String>> {
    let p = blocking(&app, |a| {
        let head = a.repo.resolve("HEAD")?;
        let now = fmt::now();
        let recent = a.repo.log(&head, None, 0, 1)?;
        let list = a.repo.tree(&head, "")?;
        let readme = README_NAMES.iter().find_map(|n| {
            a.repo
                .blob(&head, n)
                .ok()
                .and_then(|b| b.text)
                .map(|t| markdown::render_with(&t, markdown::Base::new("HEAD", "")))
        });
        let readme = readme.map(|r| r.html);
        let totals = a.records(&head)?.map(|r| {
            let t = gitterrace_index::totals(&r);
            Numbers {
                commits: fmt::thousands(t.commits),
                authors: fmt::thousands(t.authors),
                files: fmt::thousands(t.files_touched),
                additions: fmt::thousands(t.additions),
                deletions: fmt::thousands(t.deletions),
            }
        });
        Ok(OverviewPage {
            c: Chrome {
                md: readme.is_some(),
                ..views::chrome(a, "files")
            },
            totals,
            readme,
            entries: views::entries("HEAD", "", &list),
            latest: recent.first().map(|c| views::commit_row(c, now)),
        })
    })
    .await?;
    page(&p)
}

#[derive(Deserialize)]
struct LogQuery {
    path: Option<String>,
    page: Option<usize>,
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
}

async fn log(
    State(app): Shared,
    Path(rev): Path<String>,
    Query(q): Query<LogQuery>,
) -> Result<Html<String>> {
    let path = q.path.unwrap_or_default().trim_matches('/').to_owned();
    let pg = q.page.unwrap_or(0);
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let filter = (!path.is_empty()).then_some(path.as_str());
        // One extra row tells us whether an older page exists.
        let mut list = a.repo.log(&id, filter, pg.saturating_mul(PAGE), PAGE + 1)?;
        let more = list.len() > PAGE;
        list.truncate(PAGE);
        let now = fmt::now();
        let rev_enc = fmt::enc(&rev, false);
        let link = |n: usize| {
            let mut q = format!("/log/{rev_enc}?page={n}");
            if !path.is_empty() {
                crate::fmt::push(&mut q, &format!("&path={}", fmt::enc(&path, true)));
            }
            q
        };
        Ok(LogPage {
            c: views::chrome_at(a, "log", &rev),
            newer: (pg > 0).then(|| link(pg - 1)),
            older: more.then(|| link(pg + 1)),
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

struct Parent {
    full: String,
    short: String,
}

#[derive(Template)]
#[template(path = "commit.html")]
struct CommitPage {
    c: Chrome,
    id: String,
    subject: String,
    body: String,
    author: String,
    date: String,
    parents: Vec<Parent>,
    files: Vec<FileStatRow>,
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
            },
            subject: d.summary.subject.clone(),
            body: d.body.trim().to_owned(),
            author: d.summary.author.name.clone(),
            date: fmt::date(d.summary.author.time, d.summary.author.offset),
            parents: d
                .summary
                .parents
                .iter()
                .map(|p| Parent {
                    full: p.clone(),
                    short: fmt::short(p),
                })
                .collect(),
            files: views::file_stats(&d),
            diff: views::diff_html(&id, &d.files),
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
    readme: Option<String>,
}

async fn tree_root(state: Shared, Path(rev): Path<String>) -> Result<Html<String>> {
    tree(state, Path((rev, String::new()))).await
}

async fn tree(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let list = a.repo.tree(&id, &path)?;
        let rev_enc = fmt::enc(&rev, false);
        let up = path
            .rsplit_once('/')
            .map(|(parent, _)| format!("/tree/{rev_enc}/{}", fmt::enc(parent, true)));
        let up = match (up, path.is_empty()) {
            (Some(u), _) => Some(u),
            (None, false) => Some(format!("/tree/{rev_enc}")),
            (None, true) => None,
        };
        let readme = README_NAMES.iter().find_map(|n| {
            let file = if path.is_empty() {
                (*n).to_owned()
            } else {
                format!("{path}/{n}")
            };
            let text = a.repo.blob(&id, &file).ok()?.text?;
            Some(markdown::render_with(&text, markdown::Base::new(&rev, &path)).html)
        });
        Ok(TreePage {
            c: Chrome {
                md: readme.is_some(),
                ..views::chrome_at(a, "files", &rev)
            },
            readme,
            title: if path.is_empty() {
                rev.clone()
            } else {
                path.clone()
            },
            crumbs: views::crumbs(&rev, &path, false),
            entries: views::entries(&rev, &path, &list),
            path_enc: fmt::enc(&path, true),
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
    rendered: Option<String>,
}

fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".md", ".markdown", ".mdown"]
        .iter()
        .any(|e| lower.ends_with(e))
}

async fn blob(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let b = a.repo.blob(&id, &path)?;
        let (html, cut) = match b.text.as_deref() {
            Some(t) => {
                let (h, cut) = views::code_html(t);
                (Some(h), cut)
            }
            None => (None, false),
        };
        let rendered = b.text.as_deref().filter(|_| is_markdown(&path)).map(|t| {
            let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
            markdown::render_with(t, markdown::Base::new(&rev, dir)).html
        });
        let (r, pe) = (fmt::enc(&rev, false), fmt::enc(&path, true));
        Ok(BlobPage {
            c: Chrome {
                md: rendered.is_some(),
                ..views::chrome_at(a, "files", &rev)
            },
            rendered,
            crumbs: views::crumbs(&rev, &path, true),
            size: fmt::size(b.size),
            blame_href: format!("/blame/{r}/{pe}"),
            log_href: format!("/log/{r}?path={pe}"),
            title: path,
            html,
            cut,
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
        Ok(BlamePage {
            c: views::chrome_at(a, "files", &rev),
            crumbs: views::crumbs(&rev, &path, true),
            blob_href: format!("/blob/{}/{}", fmt::enc(&rev, false), fmt::enc(&path, true)),
            html: views::blame_html(&lines, fmt::now()),
            title: path,
        })
    })
    .await?;
    page(&p)
}

struct RefGroup {
    title: &'static str,
    rows: Vec<RefRow>,
}

#[derive(Template)]
#[template(path = "refs.html")]
struct RefsPage {
    c: Chrome,
    groups: Vec<RefGroup>,
}

async fn refs(State(app): Shared) -> Result<Html<String>> {
    let p = blocking(&app, |a| {
        let all = a.repo.refs()?;
        let now = fmt::now();
        let group = |title, kind| RefGroup {
            title,
            rows: views::ref_rows(&all, kind, now),
        };
        Ok(RefsPage {
            c: views::chrome(a, "refs"),
            groups: vec![
                group("Branches", RefKind::Branch),
                group("Tags", RefKind::Tag),
                group("Remotes", RefKind::Remote),
            ],
        })
    })
    .await?;
    page(&p)
}

#[derive(Template)]
#[template(path = "pending.html")]
struct PendingPage {
    c: Chrome,
    tabs: Vec<Nav>,
}

struct AuthorRow {
    label: String,
    commits: String,
    additions: String,
    deletions: String,
    detail: String,
    last: String,
    pct: u32,
}

#[derive(Template)]
#[template(path = "stats.html")]
struct StatsPage {
    c: Chrome,
    tabs: Vec<Nav>,
    activity: String,
    heatmap: String,
    authors: Vec<AuthorRow>,
}

/// Runs `build` over the cached records, or yields the progress page while none exist yet.
async fn with_records<T: Template + Send + 'static>(
    app: &Arc<App>,
    tab: &'static str,
    build: impl FnOnce(&App, &[gitterrace_index::CommitRecord], Chrome) -> T + Send + 'static,
) -> Result<Html<String>> {
    let html = blocking(app, move |a| {
        let head = a.repo.resolve("HEAD")?;
        let c = views::chrome(a, "stats");
        match a.records(&head)? {
            Some(r) => Ok(build(a, &r, c).render()?),
            None => Ok(PendingPage {
                c,
                tabs: views::stats_tabs(tab),
            }
            .render()?),
        }
    })
    .await?;
    Ok(Html(html))
}

async fn stats(State(app): Shared) -> Result<Html<String>> {
    with_records(&app, "a", |_, r, c| {
        let points = gitterrace_index::activity(r, Granularity::Month)
            .into_iter()
            .map(|b| (b.key, b.commits as u64))
            .collect::<Vec<_>>();
        let h = gitterrace_index::heatmap(r);
        let authors = gitterrace_index::authors(r);
        let max = authors.first().map_or(0, |a| a.commits as u64);
        StatsPage {
            c,
            tabs: views::stats_tabs("a"),
            activity: charts::bars(&points),
            heatmap: charts::heatmap(&h.cells, h.max),
            authors: authors
                .into_iter()
                .take(TABLE_LIMIT)
                .map(|a| AuthorRow {
                    pct: charts::pct(a.commits as u64, max),
                    label: a.name,
                    commits: fmt::thousands(a.commits),
                    additions: fmt::thousands(a.additions),
                    deletions: fmt::thousands(a.deletions),
                    detail: fmt::thousands(a.active_days),
                    last: fmt::day(a.last_commit),
                })
                .collect(),
        }
    })
    .await
}

#[derive(Template)]
#[template(path = "stats_files.html")]
struct StatsFilesPage {
    c: Chrome,
    tabs: Vec<Nav>,
    groups: Vec<RowGroup>,
}

struct RowGroup {
    title: &'static str,
    rows: Vec<Row>,
}

fn file_row(f: gitterrace_index::FileChurn, max: u64) -> Row {
    Row {
        href: format!("/blob/HEAD/{}", fmt::enc(&f.path, true)),
        pct: charts::pct(f.commits as u64, max),
        label: f.path,
        commits: fmt::thousands(f.commits),
        additions: fmt::thousands(f.additions),
        deletions: fmt::thousands(f.deletions),
    }
}

async fn stats_files(State(app): Shared) -> Result<Html<String>> {
    with_records(&app, "f", |_, r, c| {
        let rows = |v: Vec<gitterrace_index::FileChurn>| {
            let max = v.first().map_or(0, |f| f.commits as u64);
            v.into_iter()
                .take(HOT_LIMIT)
                .map(|f| file_row(f, max))
                .collect::<Vec<_>>()
        };
        let since = fmt::now() - HOT_DAYS * 86_400;
        let dirs = gitterrace_index::dir_churn(r);
        let dmax = dirs.first().map_or(0, |d| d.commits as u64);
        StatsFilesPage {
            c,
            tabs: views::stats_tabs("f"),
            groups: vec![
                RowGroup {
                    title: "Hotspots, last 90 days",
                    rows: rows(gitterrace_index::hotspots(r, since, HOT_LIMIT)),
                },
                RowGroup {
                    title: "Most changed files",
                    rows: rows(gitterrace_index::file_churn(r)),
                },
                RowGroup {
                    title: "Directories",
                    rows: dirs
                        .into_iter()
                        .take(HOT_LIMIT)
                        .map(|d| Row {
                            href: if d.path == "." {
                                String::new()
                            } else {
                                format!("/tree/HEAD/{}", fmt::enc(&d.path, true))
                            },
                            pct: charts::pct(d.commits as u64, dmax),
                            label: d.path,
                            commits: fmt::thousands(d.commits),
                            additions: fmt::thousands(d.additions),
                            deletions: fmt::thousands(d.deletions),
                        })
                        .collect(),
                },
            ],
        }
    })
    .await
}

#[derive(Template)]
#[template(path = "stats_types.html")]
struct StatsTypesPage {
    c: Chrome,
    tabs: Vec<Nav>,
    kinds: Vec<Row>,
    scopes: Vec<Row>,
    over_time: String,
}

fn label_rows(v: Vec<gitterrace_index::LabelCount>) -> Vec<Row> {
    let max = v.first().map_or(0, |l| l.commits as u64);
    v.into_iter()
        .take(TABLE_LIMIT)
        .map(|l| Row {
            pct: charts::pct(l.commits as u64, max),
            label: l.label,
            href: String::new(),
            commits: fmt::thousands(l.commits),
            additions: String::new(),
            deletions: String::new(),
        })
        .collect()
}

/// Types under 1% of typed commits fold into one `other` row. `merge` is shown as `merges`.
fn kind_rows(v: Vec<gitterrace_index::LabelCount>) -> Vec<Row> {
    let total: usize = v.iter().map(|l| l.commits).sum();
    let mut keep = Vec::new();
    let mut other = 0;
    for mut l in v {
        if l.commits * 100 < total {
            other += l.commits;
            continue;
        }
        if l.label == "merge" {
            "merges".clone_into(&mut l.label);
        }
        keep.push(l);
    }
    if other > 0 {
        keep.push(gitterrace_index::LabelCount {
            label: "other".to_owned(),
            commits: other,
        });
    }
    keep.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.label.cmp(&b.label))
    });
    label_rows(keep)
}

async fn stats_types(State(app): Shared) -> Result<Html<String>> {
    with_records(&app, "t", |_, r, c| {
        let buckets = gitterrace_index::label_over_time(r, Label::Kind, Granularity::Month);
        let over_time = charts::stacked(&buckets);
        StatsTypesPage {
            c,
            tabs: views::stats_tabs("t"),
            kinds: kind_rows(gitterrace_index::label_breakdown(r, Label::Kind)),
            scopes: label_rows(gitterrace_index::label_breakdown(r, Label::Scope)),
            over_time,
        }
    })
    .await
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
        let stats = files
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
            c: views::chrome(a, "files"),
            clean: files.is_empty(),
            diff: views::diff_html_with(None, &files),
            adds: fmt::thousands(adds),
            dels: fmt::thousands(dels),
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
#[allow(clippy::expect_used)]
mod tests {
    use crate::routes::content_type;

    #[test]
    fn content_types() {
        assert_eq!(content_type("a.PNG"), "image/png");
        assert_eq!(content_type("a.rs"), "text/plain; charset=utf-8");
    }
}
