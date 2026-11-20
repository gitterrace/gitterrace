//! The log and commit pages, with the commit filter.

use std::fmt::Write as _;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use gitterrace_index::query::{Matcher, by};
use serde::Deserialize;

use crate::filter::{Chip, ChipRow, Filter};
use crate::pagehead::{PageHead, Tab, Tagline};
use crate::routes::log_query;
use crate::routes::{PAGE, Shared, page};
use crate::{
    app::{App, blocking},
    error::Result,
    fmt,
    live::Mode,
    views,
    views::{Chrome, CommitRow, FileStatRow},
};

mod fork;

#[derive(Deserialize)]
pub(super) struct LogQuery {
    page: Option<usize>,
    view: Option<String>,
    /// `one` narrows the Graph view to this ref; the default draws every branch line.
    lanes: Option<String>,
}

pub(crate) struct DayGroup {
    pub(crate) label: String,
    pub(crate) rows: Vec<CommitRow>,
}

#[derive(Template)]
#[template(path = "log.html")]
struct LogPage {
    c: Chrome,
    days: Vec<DayGroup>,
    empty: Option<crate::empty::Empty>,
    newer: Option<String>,
    older: Option<String>,
    head: PageHead,
    /// One removable chip per filter term, and "Clear all" to the unfiltered log.
    fb: ChipRow,
    /// The Graph view's rows, one list without day headers, so the lines run unbroken.
    graph: Vec<CommitRow>,
    /// The label gutter step left of the graph for this page's labels: the class `lw{gutter}`.
    gutter: u32,
}

/// Pixels of one label character in the 11px mono label font, rounded up.
const LABEL_CH: u32 = 7;
/// Pixels a chip adds around its text: padding, border and a little air.
const LABEL_PAD: u32 = 14;
/// The gutter grows in steps of this many pixels: graph.css names each step as `lw0` to `lw12`
/// (12 steps reach its 192px cap), since the page carries no `style` attribute.
const GUTTER_STEP: u32 = 16;

/// The gutter step the labels of `rows` need, 0 when no row has one, at most 12.
fn gutter(rows: &[CommitRow]) -> u32 {
    let chars =
        rows.iter().flat_map(|r| r.refs.iter()).map(|(n, _)| n.chars().count()).max().unwrap_or(0);
    let px = match u32::try_from(chars) {
        Ok(0) | Err(_) => 0,
        Ok(n) => n.saturating_mul(LABEL_CH) + LABEL_PAD,
    };
    px.div_ceil(GUTTER_STEP).min(12)
}

/// The link to page `n` of this log, keeping its filter, view and lines.
fn page_link(rev_enc: &str, n: usize, f: &Filter, (graph, all): (bool, bool)) -> String {
    let n = n.to_string();
    let mut extra = vec![("page", n.as_str())];
    if graph {
        extra.push(("view", "graph"));
    }
    if graph && !all {
        extra.push(("lanes", "one"));
    }
    fmt::href(&format!("/log/{rev_enc}{}", f.query(&extra)))
}

/// Page `pg` of the commits reachable from any branch tip, and whether an older page exists.
fn all_lanes_page(
    a: &App,
    tips: &[String],
    pg: usize,
) -> Result<(Vec<gitterrace_git::CommitSummary>, bool)> {
    // One extra row tells us whether an older page exists.
    let mut list = a.repo.log_all(tips, pg.saturating_mul(PAGE), PAGE + 1)?;
    let more = list.len() > PAGE;
    list.truncate(PAGE);
    Ok((list, more))
}

pub(super) async fn log(
    State(app): Shared,
    Path(rev): Path<String>,
    Query(q): Query<LogQuery>,
    Query(f): Query<Filter>,
) -> Result<Html<String>> {
    // Pages past `limits.log_pages` are clamped: the walk behind a page costs its offset.
    let pg = q.page.unwrap_or(0).min(app.cfg().limits.log_pages);
    let filtered = !f.query_terms().is_empty();
    let graph = q.view.as_deref() == Some("graph") && !filtered;
    let all = graph && q.lanes.as_deref() != Some("one");
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        // The graph's lines: every branch tip, or only this ref's.
        let tips = if all { lane_tips(a, &id)? } else { vec![id.clone()] };
        let (mut list, more, capped) = if all {
            let (l, more) = all_lanes_page(a, &tips, pg)?;
            (l, more, false)
        } else {
            filtered_page(a, &id, &f, pg)?
        };
        list.shrink_to_fit();
        let now = fmt::now();
        let rev_enc = fmt::enc_seg(&rev);
        let base = format!("/log/{rev_enc}");
        let link = |n: usize| page_link(&rev_enc, n, &f, (graph, all));
        let head = log_head(&rev, &rev_enc, (graph, all), &f, &list, pg);
        let head = PageHead {
            actions: fork::since_left(a, &rev, &id).into_iter().collect(),
            toolbar: LogFilter {
                action: fmt::href(&base),
                text: f.text(),
                error: f.error().map(ToString::to_string).unwrap_or_default(),
                keywords: gitterrace_index::query::KEYWORDS
                    .iter()
                    .filter_map(|k| k.example)
                    .collect(),
                help: filter_help(),
                docs: FILTER_DOCS,
                related: related_logs(a),
            }
            .render()?,
            ..head
        };
        let graph_rows = if graph { graph_rows(a, &tips, &list, pg, now)? } else { Vec::new() };
        Ok(LogPage {
            gutter: gutter(&graph_rows),
            c: views::chrome_at(a, "log", &rev).live(Mode::NOTICE),
            head,
            newer: (pg > 0).then(|| link(pg - 1)),
            older: more.then(|| link(pg + 1)),
            fb: ChipRow { chips: chips(&f, &base, &list), clear: fmt::href(&base) },
            empty: list.is_empty().then(|| {
                let e = crate::empty::Empty::new(if capped {
                    "No match in the newest 2,000 commits."
                } else if filtered {
                    "No commits match this filter."
                } else {
                    "No commits."
                });
                if filtered { e.step("Show every commit", fmt::href(&base)) } else { e }
            }),
            graph: graph_rows,
            days: if graph { Vec::new() } else { views::day_groups(&list, now) },
        })
    })
    .await?;
    page(&p)
}

/// Page `pg` of `id`'s log under `f`, whether an older page exists, and whether the walk
/// stopped at its cap. A lone path term reads the file's history as `git log -- path` lists
/// it; any other filter goes through [`log_query::page`].
fn filtered_page(
    a: &App,
    id: &str,
    f: &Filter,
    pg: usize,
) -> Result<(Vec<gitterrace_git::CommitSummary>, bool, bool)> {
    let terms = f.query_terms();
    if terms.is_empty() {
        let mut list = a.repo.log(id, None, pg.saturating_mul(PAGE), PAGE + 1)?;
        let more = list.len() > PAGE;
        list.truncate(PAGE);
        return Ok((list, more, false));
    }
    if terms.terms().len() == 1
        && let Some(p) = f.path()
    {
        if let Some(log) = a.path_log(id, p)?
            // One past the page decides `more`; a cut list must reach it to be trusted.
            && (log.complete || log.ids.len() > pg.saturating_mul(PAGE) + PAGE)
        {
            let ids = &log.ids;
            let from = pg.saturating_mul(PAGE).min(ids.len());
            let to = (from + PAGE).min(ids.len());
            return Ok((a.repo.summaries_at(id, &ids[from..to])?, ids.len() > to, false));
        }
        let mut list = a.repo.log(id, Some(p), pg.saturating_mul(PAGE), PAGE + 1)?;
        let more = list.len() > PAGE;
        list.truncate(PAGE);
        return Ok((list, more, false));
    }
    let lp = log_query::page(a, id, &Matcher::new(terms), pg)?;
    tracing::debug!(filter = %terms, rows = lp.work.rows, walked = lp.work.walked, "log filter");
    Ok((lp.list, lp.more, lp.capped))
}

/// The filter's chips, an author's shown by the name the listed commits carry.
fn chips(f: &Filter, base: &str, list: &[gitterrace_git::CommitSummary]) -> Vec<Chip> {
    let name = |w: &str| {
        list.iter().find(|c| by(w, &c.author.name, &c.author.email)).map(|c| c.author.name.clone())
    };
    f.chips(base, None)
        .into_iter()
        .map(|mut ch| {
            if ch.dim == "author" {
                let (not, who) =
                    ch.label.strip_prefix("not ").map_or(("", ch.label.as_str()), |w| ("not ", w));
                if let Some(n) = name(who) {
                    ch.label = format!("{not}{n}");
                }
            }
            ch
        })
        .collect()
}

/// The log's toolbar, from `log_filter.html`: the one filter field (a GET form, so it works
/// without script) and the related repositories.
#[derive(Template)]
#[template(path = "log_filter.html")]
struct LogFilter {
    action: String,
    /// The filter as the field shows it: canonical, or the refused text as typed.
    text: String,
    /// Why the text was refused, `""` when it parsed.
    error: String,
    /// Example expressions the field offers as completions, from the keyword table.
    keywords: Vec<&'static str>,
    /// The help popover's lines as `(syntax, what it matches)`, from the keyword table.
    help: Vec<(&'static str, &'static str)>,
    docs: &'static str,
    /// The other served repositories sharing a root commit, as `(log href, name)`. Hrefs
    /// are built from table names only, never a path.
    related: Vec<(String, String)>,
}

/// Where the "More" link in the filter help goes: the docs page the site serves.
const FILTER_DOCS: &str = "https://gitterrace.dev/docs/using/#find-commits";

/// The filter help lines, one per keyword of the table, so a new keyword shows up here with no
/// second list to edit.
fn filter_help() -> Vec<(&'static str, &'static str)> {
    gitterrace_index::query::KEYWORDS.iter().map(|k| (k.syntax, k.description)).collect()
}

/// The log links of the other served repositories sharing a root commit, as `(href, name)`.
fn related_logs(a: &App) -> Vec<(String, String)> {
    let names = a.env.related.related(&a.key);
    names.into_iter().map(|name| (format!("/r/{}/log", fmt::enc_seg(&name)), name)).collect()
}

/// The log's page head: "Commits" with the revision, a tagline saying what the list holds,
/// and List | Graph | This ref as tabs (a filtered log has no graph, so no tabs). Graph draws
/// every branch line, as `git log --graph --all` does; This ref narrows it to the ref in the path.
fn log_head(
    rev: &str,
    rev_enc: &str,
    (graph, all): (bool, bool),
    f: &Filter,
    list: &[gitterrace_git::CommitSummary],
    pg: usize,
) -> PageHead {
    let filtered = !f.query_terms().is_empty();
    let mut said = String::from("Newest first");
    // A lone path and author say what they keep; any other filter reads from its chips.
    let (path, who) = (f.path(), f.author());
    let plain =
        f.query_terms().terms().len() == usize::from(path.is_some()) + usize::from(who.is_some());
    if filtered && !plain {
        said.push_str(", filtered");
    }
    if plain && let Some(p) = path {
        let _ = write!(said, ", touching {p}");
    }
    if plain && let Some(w) = who {
        let name = list
            .iter()
            .find(|c| by(w, &c.author.name, &c.author.email))
            .map_or(w, |c| c.author.name.as_str());
        let _ = write!(said, ", by {name}");
    }
    if pg > 0 {
        let _ = write!(said, ", page {}", pg + 1);
    }
    let _ = write!(said, " \u{b7} {} on this page", fmt::count(list.len(), "commit", "commits"));
    let tab = |label: &str, href: String, on: bool| Tab {
        label: label.to_owned(),
        href: fmt::href(&href),
        on,
    };
    PageHead {
        qualifier: rev.to_owned(),
        tabs: if filtered {
            Vec::new()
        } else {
            vec![
                tab("List", format!("/log/{rev_enc}"), !graph),
                tab("Graph", format!("/log/{rev_enc}?view=graph"), graph && all),
                tab("This ref", format!("/log/{rev_enc}?view=graph&lanes=one"), graph && !all),
            ]
        },
        ..PageHead::new("Commits", Tagline::of(&said))
    }
}

/// The tips the full graph walks: `id` (the browsed ref) first, then every local branch, then
/// each linked worktree's HEAD. A worktree HEAD is this machine's working state, so it joins
/// only where worktree pages are served (never on a public server).
fn lane_tips(a: &App, id: &str) -> Result<Vec<String>> {
    let mut tips = vec![id.to_owned()];
    for r in a.repo.refs()? {
        if r.kind == gitterrace_git::RefKind::Branch && !tips.contains(&r.target) {
            tips.push(r.target);
        }
    }
    if crate::gitstate::local_state(a) {
        for w in a.repo.linked_worktrees()? {
            if let Some(c) = w.commit.filter(|c| !tips.contains(c)) {
                tips.push(c);
            }
        }
    }
    Ok(tips)
}

/// The rows of page `pg` with their graph cells and ref chips. The layout replays the rows of
/// the pages above and reads `FAR_ROWS` past this one, commit headers only.
fn graph_rows(
    a: &App,
    tips: &[String],
    list: &[gitterrace_git::CommitSummary],
    pg: usize,
    now: i64,
) -> Result<Vec<CommitRow>> {
    use gitterrace_index::graph::FAR_ROWS;
    let from = pg.saturating_mul(PAGE);
    let to = from + list.len();
    let reach = to + FAR_ROWS as usize + 1;
    let history = a.repo.parents_all(tips, reach)?;
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
    // A detached worktree has no branch to name its tip: label it by its worktree id.
    if tips.len() > 1 && crate::gitstate::local_state(a) {
        for w in a.repo.linked_worktrees()? {
            if let (None, Some(c)) = (w.branch, w.commit) {
                refs.entry(c).or_default().push((format!("worktree {}", w.id), false));
            }
        }
    }
    Ok(list
        .iter()
        .zip(&placed)
        .enumerate()
        .map(|(i, (c, p))| {
            let mut row = views::commit_row(c, now);
            let above = i.checked_sub(1).and_then(|j| placed.get(j));
            let mut g = crate::graph_view::cell(p, above, width, c.parents.len() > 1);
            if let Some(mut r) = refs.remove(&c.id) {
                r.sort_by_key(|(_, h)| !*h);
                row.refs = r;
                g.tip = true;
            }
            row.graph = Some(g);
            row
        })
        .collect())
}

#[derive(Template)]
#[template(path = "commit.html")]
struct CommitPage {
    c: Chrome,
    head: PageHead,
    id: String,
    sha: String,
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
        return Ok(Html(h));
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
            head: PageHead::new(
                d.summary.subject.clone(),
                Tagline::of(&format!(
                    "{} \u{b7} {}",
                    fmt::count(d.files.len(), "changed file", "changed files"),
                    fmt::count(d.summary.parents.len(), "parent", "parents"),
                )),
            ),
            sha: fmt::sha_link(&id),
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
                let paths = d.files.iter().map(|f| f.path.as_str());
                let generated = gitterrace_git::generated_paths(&a.repo, &id, paths)?;
                views::diff_html_with(
                    &views::Render::of(a),
                    views::DiffSource::Commit {
                        rev: &id,
                        parent,
                        media: &media,
                        generated: &generated,
                    },
                    &d.files,
                )
            },
            id,
        })
    })
    .await?;
    let html = p.render()?;
    app.memo_put(key, &html);
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
            let new = (f.kind != ChangeKind::Deleted).then(|| read(rev, &f.path)).flatten();
            (f.path.clone(), (old, new))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use askama::Template;

    use super::{FILTER_DOCS, LogFilter, filter_help};

    fn escaped(s: &str) -> String {
        s.replace('&', "&#38;").replace('<', "&#60;").replace('>', "&#62;").replace('"', "&#34;")
    }

    #[test]
    fn filter_help_lists_every_keyword() {
        let html = LogFilter {
            action: "/log/HEAD".into(),
            text: String::new(),
            error: String::new(),
            keywords: Vec::new(),
            help: filter_help(),
            docs: FILTER_DOCS,
            related: Vec::new(),
        }
        .render()
        .expect("render");
        for k in gitterrace_index::query::KEYWORDS {
            let line = format!("<code>{}</code>", escaped(k.syntax));
            assert!(html.contains(&line), "no help line for {}: {html}", k.keyword);
        }
        assert!(html.contains("aria-label=\"Filter help\""));
        assert!(html.contains(FILTER_DOCS));
    }
}
