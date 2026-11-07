//! The log and commit pages, with the author filter.

use std::fmt::Write as _;
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use gitterrace_index::feed::{IdentId, InsightsSource, Posting, Rank};
use serde::Deserialize;

use crate::routes::{PAGE, Shared, page};
use crate::{
    app::{App, blocking},
    error::Result,
    fmt,
    live::Mode,
    views,
    views::{Chrome, CommitRow, FileStatRow},
};

#[derive(Deserialize)]
pub(super) struct LogQuery {
    path: Option<String>,
    page: Option<usize>,
    author: Option<String>,
    view: Option<String>,
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
    empty: Option<crate::empty::Empty>,
    newer: Option<String>,
    older: Option<String>,
    /// Compare from the latest tag to this revision, `""` when the page is off.
    compare_href: String,
    /// Removable chip for the active author filter: (name, href without it).
    author_chip: Option<(String, String)>,
    /// The List | Graph switch; `None` on a filtered log, which has no graph.
    views: Option<crate::seg::Seg>,
    /// The Graph view's rows, one list without day headers, so the lines run unbroken.
    graph: Vec<CommitRow>,
}

/// Commits per read of the unindexed author walk.
const CHUNK: usize = 500;
/// Commits the unindexed author walk reads before it gives up with what it has.
const AUTHOR_WALK: usize = 2_000;

/// Whether `who` (email or name) wrote the commit.
fn by(who: &str, author: &str, email: &str) -> bool {
    email.eq_ignore_ascii_case(who) || author == who
}

/// Ids of the commits by `who` touching `path`, newest first, from the author postings of the
/// index, so no history is walked.
pub(super) fn indexed_author_ids(
    src: &dyn InsightsSource,
    who: &str,
    path: Option<&str>,
) -> Vec<String> {
    let mut ranks: Vec<Rank> = (0..src.idents())
        .filter_map(|i| {
            let id = IdentId(u32::try_from(i).ok()?);
            let (name, email) = src.ident(id)?;
            by(who, &name, &email).then_some(id)
        })
        .flat_map(|id| {
            src.postings(Posting::Author(id))
                .map(|h| h.rank)
                .collect::<Vec<_>>()
        })
        .collect();
    ranks.sort_unstable();
    ranks.dedup();
    if let Some(p) = path {
        ranks = src
            .rows_for(ranks)
            .filter(|(_, rows)| {
                rows.iter().any(|f| {
                    src.path(f.path)
                        .is_some_and(|n| crate::filter::under(&n, p))
                })
            })
            .map(|(r, _)| r)
            .collect();
    }
    ranks
        .into_iter()
        .rev()
        .filter_map(|r| src.commit_id(r).map(|s| s.to_string()))
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
    if let Some(ids) = a.accelerate(id, |s| indexed_author_ids(&**s, who, path))? {
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

pub(super) async fn log(
    State(app): Shared,
    Path(rev): Path<String>,
    Query(q): Query<LogQuery>,
) -> Result<Html<String>> {
    let path = q.path.unwrap_or_default().trim_matches('/').to_owned();
    // Pages past `limits.log_pages` are clamped: the walk behind a page costs its offset.
    let pg = q.page.unwrap_or(0).min(app.cfg().limits.log_pages);
    let author = q.author;
    let filtered = !path.is_empty() || author.as_deref().is_some_and(|s| !s.is_empty());
    let graph = q.view.as_deref() == Some("graph") && !filtered;
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let filter = (!path.is_empty()).then_some(path.as_str());
        // One extra row tells us whether an older page exists.
        let who = author.as_deref().filter(|s| !s.is_empty());
        let (mut list, more) = if let Some(w) = who {
            author_page(a, &id, filter, w, pg)?
        } else if let Some(p) = filter
            && let Some(log) = a.path_log(&id, p)?
            // One past the page decides `more`; a cut list must reach it to be trusted.
            && (log.complete || log.ids.len() > pg.saturating_mul(PAGE) + PAGE)
        {
            let ids = &log.ids;
            let from = pg.saturating_mul(PAGE).min(ids.len());
            let to = (from + PAGE).min(ids.len());
            (a.repo.summaries_at(&id, &ids[from..to])?, ids.len() > to)
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
            if graph {
                q.push_str("&view=graph");
            }
            fmt::href(&q)
        };
        // Every ref is peeled to find the newest tag, so skip it when the link is not shown.
        let since = if crate::features::on(gitterrace_config::Feature::Compare) {
            crate::compare::latest_tag(&a.repo.refs()?)
        } else {
            None
        };
        Ok(LogPage {
            c: views::chrome_at(a, "log", &rev).live(Mode::NOTICE),
            compare_href: crate::compare::compare_href(since.as_deref(), &rev),
            newer: (pg > 0).then(|| link(pg - 1)),
            older: more.then(|| link(pg + 1)),
            author_chip,
            empty: list.is_empty().then(|| {
                let e = crate::empty::Empty::new("No commits.");
                if path.is_empty() && who.is_none() {
                    e
                } else {
                    e.step("Show every commit", fmt::href(&format!("/log/{rev_enc}")))
                }
            }),
            views: (!filtered).then(|| view_seg(&rev_enc, graph)),
            graph: if graph {
                graph_rows(a, &id, &list, pg, now)?
            } else {
                Vec::new()
            },
            days: if graph {
                Vec::new()
            } else {
                views::day_groups(&list, now)
            },
            rev,
            rev_enc,
            path,
        })
    })
    .await?;
    page(&p)
}

/// The List | Graph switch in the log's head.
fn view_seg(rev_enc: &str, graph: bool) -> crate::seg::Seg {
    use crate::seg::{Choice, Seg};
    let list = fmt::href(&format!("/log/{rev_enc}"));
    let g = fmt::href(&format!("/log/{rev_enc}?view=graph"));
    Seg::links(
        "View",
        vec![
            Choice::new("List", list, !graph),
            Choice::new("Graph", g, graph),
        ],
    )
    .no_caption()
}

/// The rows of page `pg` with their graph cells and ref chips. The layout replays the rows of
/// the pages above and reads `FAR_ROWS` past this one, commit headers only.
fn graph_rows(
    a: &App,
    id: &str,
    list: &[gitterrace_git::CommitSummary],
    pg: usize,
    now: i64,
) -> Result<Vec<CommitRow>> {
    use gitterrace_index::graph::FAR_ROWS;
    let from = pg.saturating_mul(PAGE);
    let to = from + list.len();
    let reach = to + FAR_ROWS as usize + 1;
    let history = a.repo.parents(id, reach)?;
    let placed = crate::graph_view::place(&history, from, to);
    let width = crate::graph_view::columns(&placed);
    let mut refs: std::collections::HashMap<String, Vec<(String, bool)>> =
        std::collections::HashMap::new();
    let head = a.repo.head_branch();
    for r in a.repo.refs()? {
        if r.kind == gitterrace_git::RefKind::Remote {
            continue;
        }
        let is_head = r.kind == gitterrace_git::RefKind::Branch && head.as_deref() == Some(&r.name);
        refs.entry(r.target).or_default().push((r.name, is_head));
    }
    Ok(list
        .iter()
        .zip(&placed)
        .enumerate()
        .map(|(i, (c, p))| {
            let mut row = views::commit_row(c, now);
            let above = i.checked_sub(1).and_then(|j| placed.get(j));
            row.graph = Some(crate::graph_view::cell(
                p,
                above,
                width,
                c.parents.len() > 1,
            ));
            if let Some(mut r) = refs.remove(&c.id) {
                r.sort_by_key(|(_, h)| !*h);
                row.refs = r;
            }
            row
        })
        .collect())
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
    /// Lines added and removed over all files, shown on the right of the summary.
    total_add: String,
    total_del: String,
    files: Vec<FileStatRow>,
    /// `(label, url)`: "Find this commit in ..." links from the `lookup` of each Send to target.
    lookups: Vec<(String, String)>,
    diff: String,
}

/// A commit never changes, so its page (diff and diffstat included) is rendered once per commit.
/// The memo key still moves with HEAD and config, which shape the chrome and the diff folding.
pub(super) async fn commit(State(app): Shared, Path(id): Path<String>) -> Result<Html<String>> {
    let (key, id) = blocking(&app, move |a| {
        let id = a.repo.resolve(&id)?;
        Ok((a.memo_key("commit", Some(&id)), id))
    })
    .await?;
    if let Some(h) = app.memo_get(&key) {
        return Ok(Html((*h).clone()));
    }
    let p = blocking(&app, move |a| {
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
            total_add: fmt::thousands(d.files.iter().map(|f| u64::from(f.additions)).sum::<u64>()),
            total_del: fmt::thousands(d.files.iter().map(|f| u64::from(f.deletions)).sum::<u64>()),
            files: views::file_stats(&d),
            lookups: a.lookups(&id),
            diff: {
                let parent = d.summary.parents.first().map(String::as_str);
                let media = media_facts(a, &id, parent, &d.files);
                views::diff_html_with(
                    &views::Render::of(a),
                    views::DiffSource::Commit {
                        rev: &id,
                        parent,
                        media: &media,
                    },
                    &d.files,
                )
            },
            id,
        })
    })
    .await?;
    let html = p.render()?;
    app.memo_put(key, Arc::new(html.clone()));
    Ok(Html(html))
}

/// Bytes and pixel size of both sides of each changed image, for its diff header. Only the
/// image files of the commit are read, and a side that cannot be read is left out.
fn media_facts(
    a: &App,
    rev: &str,
    parent: Option<&str>,
    files: &[gitterrace_git::FileDiff],
) -> views::MediaFacts {
    use gitterrace_git::ChangeKind;
    // Like the file page, only images under `limits.preview_mb` are read for their pixel size.
    let cap = a.cfg().limits.preview_mb << 20;
    let read = |at: &str, path: &str| {
        let len = a.repo.blob(at, path).ok()?.size;
        if len > cap {
            return Some(crate::media::Facts { len, dims: None });
        }
        let b = a.repo.blob_bytes(at, path).ok()?;
        Some(crate::media::Facts::of(path, &b))
    };
    files
        .iter()
        .filter(|f| f.binary && crate::media::is_image(&f.path))
        .map(|f| {
            let old_path = f.old_path.as_deref().unwrap_or(&f.path);
            let old = (f.kind != ChangeKind::Added)
                .then_some(parent)
                .flatten()
                .and_then(|p| read(p, old_path));
            let new = (f.kind != ChangeKind::Deleted)
                .then(|| read(rev, &f.path))
                .flatten();
            (f.path.clone(), (old, new))
        })
        .collect()
}
