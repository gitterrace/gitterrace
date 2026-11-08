//! Route handlers. Each resolves its revision first, then reads only from that commit id.

use std::sync::Arc;

use askama::Template;
use axum::{
    Json, Router,
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use gitterrace_git::RefKind;
use serde::Deserialize;

use crate::{
    app::{App, blocking},
    empty::Empty,
    error::Result,
    fmt,
    live::Mode,
    views,
    views::{Chrome, CommitRow, FileStatRow},
};

mod api;
mod assets;
mod commits;
mod files;
mod raw;

pub(crate) use assets::assets;
pub use assets::font_bytes;
pub(crate) use commits::DayGroup;
#[cfg(test)]
use commits::indexed_author_ids;
use commits::{commit, log};
pub(crate) use files::prewarm_blame;
use files::{blame, blob, highlight_tail, tree, tree_root};
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
        .route("/worktree/raw/{*path}", get(raw::worktree_raw))
        .route("/api/files/{rev}", get(api::files))
        .route("/api/refs", get(api::refs))
        .route("/api/last/{rev}", get(api::last_root))
        .route("/api/last/{rev}/{*path}", get(api::last))
        .route("/stats", get(crate::activity::page))
        // The tab reads "Insights": a typed or remembered /insights lands there too.
        .route(
            "/insights",
            get(|State(a): Shared| async move { Redirect::to(&format!("{}/stats", a.base)) }),
        )
        .route("/stats/types", get(moved))
        .route("/stats/languages", get(crate::languages::page))
        .route("/stats/frequency", get(moved))
        .route("/stats/hotspots", get(crate::hotspots::page))
        .route("/stats/ownership", get(crate::ownership::page))
        .route("/stats/patterns", get(moved))
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
        .with_state(Arc::clone(&app))
        .layer(axum::middleware::from_fn_with_state(
            app,
            crate::lost::in_shell,
        ))
}

/// Makes `?rev=` the selected ref for everything the request renders.
async fn selected_rev(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let rev = fmt::rev_param(req.uri().query());
    fmt::REQUEST_REV.scope(rev, next.run(req)).await
}

/// Insights pages that became sections of another page: the old path, the new one and the
/// section it lands on.
pub(crate) const MOVED_PAGES: [(&str, &str, &str); 3] = [
    ("/stats/types", "/stats", "types"),
    ("/stats/frequency", "/stats", "frequency"),
    ("/stats/patterns", "/stats/hotspots", "patterns"),
];

/// Where an old insights path lives now, with its query (period, chips, options) kept and the
/// section as the fragment.
pub(crate) fn moved_target(path: &str, query: Option<&str>) -> Option<String> {
    let (_, to, anchor) = MOVED_PAGES.iter().find(|(from, _, _)| *from == path)?;
    let q = query
        .filter(|q| !q.is_empty())
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    Some(format!("{to}{q}#{anchor}"))
}

/// A permanent redirect from a page that moved into a section of another page.
async fn moved(State(app): Shared, uri: axum::http::Uri) -> Response {
    let path = uri.path();
    let own = MOVED_PAGES
        .iter()
        .map(|m| m.0)
        .find(|p| path.ends_with(p))
        .unwrap_or_default();
    moved_target(own, uri.query()).map_or_else(
        || axum::http::StatusCode::NOT_FOUND.into_response(),
        |to| Redirect::permanent(&format!("{}{to}", app.base)).into_response(),
    )
}

fn page<T: Template>(t: &T) -> Result<Html<String>> {
    Ok(Html(t.render()?))
}

struct Numbers {
    /// `data-population` of the commit and merge counts: every commit.
    every: &'static str,
    /// `data-population` of the authors, files and lines: the counted commits.
    counted: &'static str,
    commits: String,
    commits_noun: &'static str,
    /// "(2 merges, 3 counted)" or empty: the overview counts every commit, the rest of the
    /// line and the insights only the counted ones.
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
    head: crate::pagehead::PageHead,
    totals: Option<Numbers>,
    about: Option<crate::about::About>,
    entries: Vec<views::Entry>,
    latest: Option<CommitRow>,
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
        // A failed read leaves out the numbers (the cache is rebuilt behind them); the code
        // part of the overview needs no cache and still shows.
        let built = match a.source(&head)? {
            Some(src) => src
                .run(|src| {
                    let side = crate::overview_side::html(a, &head, &**src);
                    (side, Some(overview_numbers(a, &head, src)))
                })
                .or_else(|e| a.unreadable(&e).map(|()| (String::new(), None)))?,
            None => (String::new(), None),
        };
        let (side, totals) = built;
        let c = views::chrome(a, "files").live(Mode::NOTICE);
        let counted = totals.as_ref().map(|t| {
            format!(
                "{} {} by {} {}",
                t.commits, t.commits_noun, t.authors, t.authors_noun
            )
        });
        Ok(OverviewPage {
            head: crate::pagehead::overview_head(&c.name, &c.branch, counted.as_deref()),
            c: Chrome {
                md: about.as_ref().is_some_and(|d| d.markdown),
                ..c
            },
            totals,
            about,
            entries: views::entries(&rev, "", &list),
            latest: recent.first().map(|c| views::commit_row(c, now)),
            side,
            no_side: a.cfg().display.sidebar.is_empty(),
        })
    })
    .await?;
    page(&p)
}

/// "(1 merge, 2 by bots, 3 counted)": what sets every commit apart from the counted ones, and
/// how many those are. Empty when every commit counts. Bots are named only when the options
/// leave them out.
fn apart(merges: u64, bots: u64, counted: usize, every: u64, bots_count: bool) -> String {
    if counted as u64 == every {
        return String::new();
    }
    let mut parts = Vec::new();
    if merges > 0 {
        parts.push(fmt::count(merges, "merge", "merges"));
    }
    if bots > 0 && !bots_count {
        parts.push(format!("{} by bots", fmt::thousands(bots)));
    }
    parts.push(format!("{} counted", fmt::thousands(counted)));
    format!("({})", parts.join(", "))
}

/// The overview's totals line over `src`.
fn overview_numbers(a: &App, head: &str, src: &crate::insights::Src) -> Numbers {
    let all = a.totals(head, &**src, a.defaults().into_options());
    let (e, t) = (all.every, all.counted);
    let (commits, merges) = (e.commits.value(), e.merges.value());
    Numbers {
        every: e.commits.population().as_str(),
        counted: t.commits_figure().population().as_str(),
        commits: fmt::thousands(commits),
        commits_noun: fmt::noun(commits, "commit", "commits"),
        merges: apart(
            merges,
            e.bots.value(),
            t.commits,
            commits,
            a.defaults().bots,
        ),
        authors: fmt::thousands(t.authors),
        authors_noun: fmt::noun(t.authors, "author", "authors"),
        files: fmt::thousands(t.files),
        files_noun: fmt::noun(t.files, "file touched", "files touched"),
        additions: fmt::thousands(t.additions),
        deletions: fmt::thousands(t.deletions),
        lines_noun: fmt::noun(t.additions + t.deletions, "line", "lines"),
    }
}

#[derive(Template)]
#[template(path = "refs.html")]
struct RefsPage {
    c: Chrome,
    head: crate::pagehead::PageHead,
    branches: Vec<CommitRow>,
    tags: Vec<CommitRow>,
    remotes: Vec<CommitRow>,
    worktrees: Vec<crate::gitstate::WorktreeRow>,
    stashes: Vec<crate::gitstate::StashRow>,
    no_branches: Option<Empty>,
    /// Set only when the whole page has nothing to show.
    nothing: Option<Empty>,
    branches_cut: Option<RefsCut>,
    tags_cut: Option<RefsCut>,
    remotes_cut: Option<RefsCut>,
}

/// Rows each section of `/refs` renders unless asked for all. A row reads its commit (and a
/// branch walks its divergence), so a repository with thousands of tags took seconds; the
/// newest rows are the ones a visitor looks for, and `?all=1` lists every one.
pub(crate) const REFS_SHOWN: usize = 100;

#[derive(Deserialize)]
struct RefsQuery {
    all: Option<String>,
}

/// A section cut at [`REFS_SHOWN`]: how many exist, and the link to all of them.
struct RefsCut {
    said: String,
    href: String,
    label: String,
}

async fn refs(State(app): Shared, Query(q): Query<RefsQuery>) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let all = a.repo.refs()?;
        let now = fmt::now();
        let limit = if q.all.as_deref() == Some("1") {
            usize::MAX
        } else {
            REFS_SHOWN
        };
        let total = |k: RefKind| all.iter().filter(|r| r.kind == k).count();
        let tags = views::ref_rows(a, &all, RefKind::Tag, now, limit)?;
        let branches = crate::gitstate::branch_rows(a, &all, limit)?;
        let remotes = views::ref_rows(a, &all, RefKind::Remote, now, limit)?;
        let cut = |k: RefKind, shown: usize, one: &'static str, many: &'static str| {
            let n = total(k);
            (n > shown).then(|| RefsCut {
                said: format!(
                    "{}, {} shown.",
                    fmt::count(n, one, many),
                    fmt::thousands(shown)
                ),
                href: fmt::href("/refs?all=1"),
                label: format!("Show all {}", fmt::count(n, one, many)),
            })
        };
        let branches_cut = cut(RefKind::Branch, branches.len(), "branch", "branches");
        let tags_cut = cut(RefKind::Tag, tags.len(), "tag", "tags");
        let remotes_cut = cut(
            RefKind::Remote,
            remotes.len(),
            "remote branch",
            "remote branches",
        );
        let (worktrees, stashes) = if crate::gitstate::local_state(a) {
            (
                crate::gitstate::worktree_rows(a)?,
                crate::gitstate::stash_rows(a)?,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let nothing = (branches.is_empty()
            && tags.is_empty()
            && remotes.is_empty()
            && worktrees.is_empty()
            && stashes.is_empty())
        .then(|| Empty::new("No branches, tags or remotes."));
        Ok(RefsPage {
            c: views::chrome(a, "refs").live(Mode::REFS),
            head: crate::pagehead::refs_head(total),
            no_branches: branches.is_empty().then(|| Empty::new("No branches.")),
            nothing,
            branches,
            tags,
            remotes,
            worktrees,
            stashes,
            branches_cut,
            tags_cut,
            remotes_cut,
        })
    })
    .await?;
    page(&p)
}

#[derive(Template)]
#[template(path = "pending.html")]
struct PendingPage {
    c: Chrome,
    page_head: crate::pagehead::PageHead,
    /// Set when a newer build owns the cache: this one leaves it alone instead of indexing.
    newer: Option<String>,
    head: crate::dochead::DocHead,
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
        // The page is rendered inside the check: a storage error meanwhile refuses it, so it
        // is neither shown nor memoized, and the cache is rebuilt behind the indexing page.
        if let Some(src) = a.source(&head)? {
            match src.run(|s| build(a, s, c).render()) {
                Ok(html) => return Ok((html?, true)),
                Err(e) => a.unreadable(&e)?,
            }
        }
        let c = views::chrome(a, "stats").live(Mode::NOTICE);
        if let Some(read) = a.far(&head) {
            return Ok((crate::far::render(c, tab, read)?, false));
        }
        Ok((
            PendingPage {
                c,
                page_head: crate::pagehead::waiting_head(tab),
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
                head: crate::dochead::DocHead {
                    title: "Indexing history".to_owned(),
                    ..crate::dochead::DocHead::default()
                },
            }
            .render()?,
            false,
        ))
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
        // Taken before the diff: an edit in between shows once more, never not at all. The
        // oldest render not yet compared stays: a newer one may already hold an edit an older
        // tab has not seen.
        let sig = gitterrace_git::worktree_signature(&a.repo).ok();
        if let Ok(mut g) = a.wt_rendered.lock()
            && g.is_none()
        {
            *g = sig;
        }
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
            diff: views::diff_html_with(&views::Render::of(a), views::DiffSource::Worktree, &files),
            adds: fmt::thousands(adds),
            dels: fmt::thousands(dels),
            files_summary: fmt::count(stats.len(), "changed file", "changed files"),
            files: stats,
        })
    })
    .await?;
    page(&p)
}

#[cfg(test)]
mod tests;
