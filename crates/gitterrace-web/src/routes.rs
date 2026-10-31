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
use gitterrace_index::{Granularity, Label, Totals};

use crate::{
    app::{App, blocking},
    charts,
    error::Result,
    fmt, markdown, views,
    views::{Chrome, CommitRow, Crumb, Entry, FileStatRow, Nav, RefRow, Row},
};

const PAGE: usize = 50;
const RECENT: usize = 8;
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
        .route("/stats", get(stats))
        .route("/stats/files", get(stats_files))
        .route("/stats/types", get(stats_types))
        .route("/api/progress", get(progress))
        .route(
            "/static/app.css",
            get(|| asset("text/css; charset=utf-8", include_str!("../static/app.css"))),
        )
        .route(
            "/static/app.js",
            get(|| {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../static/app.js"),
                )
            }),
        )
        .with_state(app)
}

async fn asset(kind: &'static str, body: &'static str) -> Response {
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

#[derive(Template)]
#[template(path = "overview.html")]
struct OverviewPage {
    c: Chrome,
    totals: Option<Totals>,
    readme: Option<String>,
    recent: Vec<CommitRow>,
}

async fn overview(State(app): Shared) -> Result<Html<String>> {
    let p = blocking(&app, |a| {
        let head = a.repo.resolve("HEAD")?;
        let now = fmt::now();
        let recent = a.repo.log(&head, None, 0, RECENT)?;
        let readme = README_NAMES.iter().find_map(|n| {
            a.repo
                .blob(&head, n)
                .ok()
                .and_then(|b| b.text)
                .map(|t| markdown::render(&t))
        });
        let totals = a.records(&head)?.map(|r| gitterrace_index::totals(&r));
        Ok(OverviewPage {
            c: views::chrome(&a.name, "overview"),
            totals,
            readme,
            recent: recent.iter().map(|c| views::commit_row(c, now)).collect(),
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

#[derive(Template)]
#[template(path = "log.html")]
struct LogPage {
    c: Chrome,
    rev: String,
    rev_enc: String,
    path: String,
    rows: Vec<CommitRow>,
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
                q.push_str(&format!("&path={}", fmt::enc(&path, true)));
            }
            q
        };
        Ok(LogPage {
            c: views::chrome(&a.name, "log"),
            newer: (pg > 0).then(|| link(pg - 1)),
            older: more.then(|| link(pg + 1)),
            rows: list.iter().map(|c| views::commit_row(c, now)).collect(),
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
            c: views::chrome(&a.name, "log"),
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
        Ok(TreePage {
            c: views::chrome(&a.name, "files"),
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
        let (r, pe) = (fmt::enc(&rev, false), fmt::enc(&path, true));
        Ok(BlobPage {
            c: views::chrome(&a.name, "files"),
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
            c: views::chrome(&a.name, "files"),
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
            c: views::chrome(&a.name, "refs"),
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
    commits: usize,
    additions: u64,
    deletions: u64,
    detail: usize,
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
        let c = views::chrome(&a.name, "stats");
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
                    commits: a.commits,
                    additions: a.additions,
                    deletions: a.deletions,
                    detail: a.active_days,
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
        commits: f.commits,
        additions: f.additions,
        deletions: f.deletions,
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
                            commits: d.commits,
                            additions: d.additions,
                            deletions: d.deletions,
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
            commits: l.commits,
            additions: 0,
            deletions: 0,
        })
        .collect()
}

async fn stats_types(State(app): Shared) -> Result<Html<String>> {
    with_records(&app, "t", |_, r, c| {
        let buckets = gitterrace_index::label_over_time(r, Label::Kind, Granularity::Month);
        let points = buckets
            .iter()
            .map(|b| {
                (
                    b.key.clone(),
                    b.counts.iter().map(|c| c.commits as u64).sum(),
                )
            })
            .collect::<Vec<_>>();
        StatsTypesPage {
            c,
            tabs: views::stats_tabs("t"),
            kinds: label_rows(gitterrace_index::label_breakdown(r, Label::Kind)),
            scopes: label_rows(gitterrace_index::label_breakdown(r, Label::Scope)),
            over_time: charts::bars(&points),
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
