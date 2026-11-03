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
use gitterrace_git::RefKind;
use serde::Deserialize;

use crate::{
    app::{App, blocking},
    error::Result,
    fmt,
    live::Mode,
    views,
    views::{Chrome, CommitRow, FileStatRow, RefRow},
};

mod assets;
mod commits;
mod files;
mod raw;

pub(crate) use assets::assets;
pub(crate) use commits::DayGroup;
#[cfg(test)]
use commits::indexed_author_ids;
use commits::{commit, log};
pub(crate) use files::prewarm_blame;
use files::{blame, blob, highlight_tail, tree, tree_root};
pub(crate) use raw::MAX_PREVIEW_BYTES;
use raw::raw;
#[cfg(test)]
pub(crate) use raw::raw_refused;

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
        .route("/worktree/diff/{*path}", get(views::worktree_fragment))
        .route("/api/files/{rev}", get(api_files))
        .route("/api/refs", get(api_refs))
        .route("/api/last/{rev}", get(api_last_root))
        .route("/api/last/{rev}/{*path}", get(api_last))
        .route("/stats", get(crate::activity::page))
        .route("/stats/types", get(crate::commit_types::page))
        .route("/stats/languages", get(crate::languages::page))
        .route("/stats/frequency", get(crate::insights::frequency))
        .route("/stats/hotspots", get(crate::hotspots::page))
        .route("/stats/ownership", get(crate::ownership::page))
        .route("/stats/patterns", get(crate::patterns::page))
        .route("/compare", get(crate::compare::page))
        .route("/history/{*path}", get(crate::history::page))
        .route("/stats/contributors", get(crate::insights::contributors))
        .route("/raw/{rev}/{*path}", get(raw))
        .route("/api/progress", get(progress))
        .route("/worktrees", get(crate::gitstate::worktrees))
        .route("/stashes", get(crate::gitstate::stashes))
        .route("/tag/{name}", get(crate::gitstate::tag))
        .route("/events", get(crate::live::events))
        .layer(axum::middleware::from_fn(selected_rev))
        .merge(assets())
        .fallback(crate::error::fallback)
        .with_state(app)
}

/// Makes `?rev=` the selected ref for everything the request renders.
async fn selected_rev(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let rev = fmt::rev_param(req.uri().query());
    fmt::REQUEST_REV.scope(rev, next.run(req)).await
}

fn page<T: Template>(t: &T) -> Result<Html<String>> {
    Ok(Html(t.render()?))
}

struct Numbers {
    commits: String,
    commits_noun: &'static str,
    /// "(2 merges)" or empty: the overview counts every commit, insights leave merges out.
    merges: String,
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
    /// The memoized sidebar sections; empty while indexing.
    side: String,
    /// `display.sidebar` is empty: no sidebar, the files take the full width.
    no_side: bool,
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
        let src = a.source(&head)?;
        let side = src
            .as_ref()
            .map(|s| crate::overview_side::html(a, &head, &**s))
            .unwrap_or_default();
        let totals = src.map(|src| {
            let t = a.totals(&head, &*src, a.defaults().into_options());
            Numbers {
                commits: fmt::thousands(src.len()),
                commits_noun: fmt::noun(src.len(), "commit", "commits"),
                merges: match crate::overview_side::merges(a, &head, &*src) {
                    0 => String::new(),
                    n => format!("({})", fmt::count(n, "merge", "merges")),
                },
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
            side,
            no_side: a.cfg().display.sidebar.is_empty(),
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

#[derive(Template)]
#[template(path = "pending.html")]
struct PendingPage {
    c: Chrome,
    tabs: views::Tabs,
    /// Set when a newer build owns the cache: this one leaves it alone instead of indexing.
    newer: Option<String>,
}

/// Runs `build` over the tip's insights source, or yields the progress page while none exist
/// yet. The flag says the page was built from a source: only such a page may be memoized, as
/// the indexing page can be rendered just before the index turns ready and would then be
/// served after it.
pub(crate) async fn with_records<T: Template + Send + 'static>(
    app: &Arc<App>,
    tab: &'static str,
    build: impl FnOnce(&App, &crate::insights::Src, Chrome) -> T + Send + 'static,
) -> Result<(Html<String>, bool)> {
    let (html, served) = blocking(app, move |a| {
        let head = a.repo.resolve(&fmt::selected())?;
        let c = views::chrome(a, "stats").live(Mode::NOTICE);
        match a.source(&head)? {
            Some(src) => Ok((build(a, &src, c).render()?, true)),
            None if a.far(&head).is_some() => Ok((
                crate::far::render(c, tab, a.far(&head).unwrap_or(0))?,
                false,
            )),
            None => Ok((
                PendingPage {
                    c,
                    tabs: views::stats_tabs(tab),
                    newer: a.index.newer_format().map(|v| {
                        // The cache folder is a path on the host; a public visitor gets no path.
                        let dir = if a.public() {
                            "its cache folder".to_owned()
                        } else {
                            a.index.dir().display().to_string()
                        };
                        format!(
                            "The history cache was written by a newer gitterrace (format {v}, this \
                         build reads {}). It is left untouched: run the newer build, or delete \
                         {dir} to rebuild it here.",
                            gitterrace_index::FORMAT_VERSION,
                        )
                    }),
                }
                .render()?,
                false,
            )),
        }
    })
    .await?;
    Ok((Html(html), served))
}

#[derive(serde::Serialize)]
struct ProgressBody {
    done: usize,
    total: usize,
    ready: bool,
    /// Idle releases so far, see `App::release_idle`.
    releases: u64,
}

async fn progress(State(app): Shared) -> Json<ProgressBody> {
    let p = app.progress();
    Json(ProgressBody {
        done: p.done,
        total: p.total,
        ready: app.ready(),
        releases: app.releases(),
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
            diff: views::diff_html_with(
                a.cfg().limits.diff_fold_lines,
                views::DiffSource::Worktree,
                &files,
            ),
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
mod tests;
