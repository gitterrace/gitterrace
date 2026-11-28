//! The tree, blob and blame pages, and the blame warm-up.

use std::fmt::Write as _;
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use gitterrace_config::Feature;
use gitterrace_index::feed::{CommitRange, InsightsSource, Rank};

use crate::guard::Styled;
use crate::pagehead::{Action, FileView, PageHead, Tagline};
use crate::routes::{DocQuery, Shared};
use crate::{
    app::{App, blocking},
    error::Result,
    fmt, markdown, views,
    views::{Chrome, Entry},
};

#[derive(Template)]
#[template(path = "tree.html")]
struct TreePage {
    c: Chrome,
    title: String,
    head: PageHead,
    path_enc: String,
    up: Option<String>,
    entries: Vec<Entry>,
    about: Option<crate::about::About>,
    /// The badge fragment of this folder, empty when insights are off.
    card_href: String,
}

pub(super) async fn tree_root(
    state: Shared,
    Path(rev): Path<String>,
    q: Query<DocQuery>,
) -> Result<Styled> {
    tree(state, Path((rev, String::new())), q).await
}

/// A tree at one commit never changes, so its page (README card included) is rendered once per
/// (rev, commit, path, doc). The memo key still moves with HEAD, the index and config.
pub(super) async fn tree(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
    Query(q): Query<DocQuery>,
) -> Result<Styled> {
    let (key, rev, path, q) = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let query = format!(
            "{rev}\0{id}\0{}\0{}",
            path.trim_matches('/'),
            q.doc.as_deref().unwrap_or_default()
        );
        Ok((a.memo_key("tree", Some(&query)), rev, path, q))
    })
    .await?;
    if let Some(h) = app.memo_get(&key) {
        return Ok(Styled(h));
    }
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
            card_href: crate::card::href(&rev, &path),
            about,
            title: if path.is_empty() { rev.clone() } else { path.clone() },
            head: tree_head(&rev, &path, &list),
            entries: views::entries(&rev, &path, &list),
            path_enc: fmt::enc_path(&path),
            up,
        })
    })
    .await?;
    let html = p.render()?;
    app.memo_put(key, &html);
    Ok(Styled(html))
}

/// A folder's page head: its path as the title, what it holds as the tagline, and its history
/// as the action.
fn tree_head(rev: &str, path: &str, list: &[gitterrace_git::TreeEntry]) -> PageHead {
    let dirs = list.iter().filter(|e| e.kind == gitterrace_git::TreeEntryKind::Dir).count();
    let files = list.len() - dirs;
    let said = match (dirs, files) {
        (0, 0) => "An empty folder".to_owned(),
        (0, f) => fmt::count(f, "file", "files"),
        (d, 0) => fmt::count(d, "folder", "folders"),
        (d, f) => {
            format!("{} and {}", fmt::count(d, "folder", "folders"), fmt::count(f, "file", "files"))
        }
    };
    let r = fmt::enc_seg(rev);
    let mut actions = vec![Action {
        label: "History",
        href: crate::filter::Filter::default().link(
            &format!("/log/{r}"),
            crate::filter::Dim::Path,
            path,
        ),
    }];
    // A sub folder offers a code search scoped to it: the search form opens with its path glob
    // filled in. The root needs none, the header search already covers it.
    let search = crate::features::href_if(
        Feature::Search,
        &format!("/search?rev={r}&glob={}", fmt::enc_seg(&format!("{path}/"))),
    );
    if !path.is_empty() && !search.is_empty() {
        actions.push(Action { label: "Search here", href: search });
    }
    PageHead {
        crumbs: views::crumbs(rev, path, false),
        actions,
        ..PageHead::new(path, Tagline::of(&said))
    }
}

#[derive(Template)]
#[template(path = "blob.html")]
struct BlobPage {
    c: Chrome,
    title: String,
    page_head: PageHead,
    size: String,
    html: Option<String>,
    cut: bool,
    /// The cut notice, naming `limits.blob_lines`.
    cut_note: String,
    media: Option<String>,
    /// An archive's entry list (`filetype::archive::html`, escaped there).
    archive: Option<String>,
    /// A font's facts and sample (`filetype::font::html`, escaped there).
    font: Option<String>,
    too_large: bool,
    raw_href: String,
    /// The format of a binary blob the page does not preview; empty otherwise.
    format: &'static str,
    rendered: Option<String>,
    /// "On this page" as the rail: the document's headings, or a source file's symbols.
    outline_rail: String,
    /// The card fragment of this file, empty for the working tree or without insights.
    card_href: String,
}

fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".md", ".markdown", ".mdown"].iter().any(|e| lower.ends_with(e))
}

/// The highlighted chunks a big blob's first paint left plain; `nav.js` swaps them in.
pub(super) async fn highlight_tail(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Html<String>> {
    let html = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let text = a.repo.blob(&id, &path)?.text.unwrap_or_default();
        Ok(views::code_tail_html(&views::Render::of(a), &path, &text))
    })
    .await?;
    Ok(Html(html))
}

pub(super) async fn blob(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Styled> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        // One content read serves the text and, for media small enough to want facts about,
        // the bytes.
        let (b, bytes) =
            if crate::media::kind(&path).is_some() || crate::filetype::wants_bytes(&path) {
                a.repo.blob_with_bytes(&id, &path, a.cfg().limits.preview_mb << 20)?
            } else {
                (a.repo.blob(&id, &path)?, Vec::new())
            };
        Ok(blob_page(a, BlobAt::Rev(&rev), path, &b, &bytes, Some(id.as_str())))
    })
    .await?;
    Ok(Styled(p.render()?))
}

/// `/worktree/blob/{path}`: the blob view over one changed file as it is on disk, read only
/// through `gitterrace-git`'s worktree module under `limits.raw_mb`. Not found for anything
/// that is not a changed file; a public server never routes here (`public.rs`).
pub(super) async fn worktree_blob(
    State(app): Shared,
    Path(path): Path<String>,
) -> Result<axum::response::Response> {
    use axum::response::IntoResponse as _;
    let path = path.trim_matches('/').to_owned();
    let p = blocking(&app, move |a| {
        let limit = a.cfg().limits.raw_mb << 20;
        Ok(gitterrace_git::worktree_blob(&a.repo, &path, limit)?
            .map(|(b, bytes)| blob_page(a, BlobAt::Worktree, path, &b, &bytes, None)))
    })
    .await?;
    match p {
        Some(p) => Ok(Styled(p.render()?).into_response()),
        None => Ok(crate::error::not_found("This file is not a changed file of the working tree.")),
    }
}

/// Where a blob page's content was read: a revision, or the working tree on disk.
#[derive(Clone, Copy)]
enum BlobAt<'a> {
    Rev(&'a str),
    Worktree,
}

/// The blob page's raw link and chrome: the revision's `/raw`, or the file on disk.
fn blob_links(a: &App, at: BlobAt<'_>, path: &str) -> (String, Chrome) {
    let pe = fmt::enc_path(path);
    match at {
        BlobAt::Rev(r) => (
            crate::features::href_if(Feature::Raw, &format!("/raw/{}/{pe}", fmt::enc_seg(r))),
            views::chrome_at(a, "files", r),
        ),
        BlobAt::Worktree => (fmt::href(&format!("/worktree/raw/{pe}")), views::chrome(a, "files")),
    }
}

/// The blob page for content already read: highlight, outline, Markdown preview and media.
fn blob_page(
    a: &App,
    at: BlobAt<'_>,
    path: String,
    b: &gitterrace_git::Blob,
    bytes: &[u8],
    sha: Option<&str>,
) -> BlobPage {
    // Relative Markdown links and the chrome's revision resolve against HEAD for the worktree.
    let rev = match at {
        BlobAt::Rev(r) => r,
        BlobAt::Worktree => "HEAD",
    };
    let media = crate::media::kind(&path).map(|k| {
        let src = (k == crate::media::Kind::Svg)
            .then(|| std::str::from_utf8(bytes).ok())
            .flatten()
            .filter(|t| t.len() as u64 <= a.repo.text_limit());
        let side = match at {
            BlobAt::Rev(r) => crate::media::Side::At(r, &path),
            BlobAt::Worktree => crate::media::Side::Worktree(&path),
        };
        crate::media::blob_html(k, b.size, side, bytes, src)
    });
    let too_large = b.size > a.repo.text_limit();
    let text = b.text.as_deref().filter(|_| !too_large);
    // The worktree has no `/hl` tail route, so its file highlights whole on first paint.
    let code = text.map(|t| match at {
        BlobAt::Rev(_) => views::code_first(&views::Render::of(a), &path, t),
        BlobAt::Worktree => views::code(&views::Render::of(a), &path, t),
    });
    let cut = code.as_ref().is_some_and(|c| c.cut);
    let (html, named) = code.map_or((None, Vec::new()), |c| (Some(c.html), c.symbols));
    // The marks are drawn on the page, never in the shared highlight memo.
    let html = html.map(|h| crate::blob_marks::apply(h, &crate::blob_marks::marks(a, sha, &path)));
    let preview = crate::features::on(Feature::MarkdownPreview);
    let doc = text.filter(|_| preview && is_markdown(&path)).map(|t| {
        let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
        markdown::render_doc(
            t,
            markdown::Base::new(rev, dir).highlight(views::Render::of(a).highlight_bytes),
        )
    });
    let doc = doc.or_else(|| markdown::notebook::doc(text?, &path, rev, views::Render::of(a)));
    let table = crate::filetype::table_html(&path, text.filter(|_| doc.is_none()), bytes);
    let archive = crate::filetype::archive_html(&path, bytes);
    let (raw_href, chrome) = blob_links(a, at, &path);
    let font = crate::filetype::font::html(&path, bytes, &raw_href);
    let symbols = match (&doc, text, media.is_some()) {
        (None, Some(t), false) => {
            crate::symbols::outline(&path, t, views::Render::of(a).blob_lines, named)
        }
        _ => Vec::new(),
    };
    let outline_rail = doc
        .as_ref()
        .map_or_else(|| markdown::outline_rail(&symbols), |d| markdown::outline_rail(&d.outline));
    let mut said = fmt::size(b.size);
    if let Some(t) = text {
        let _ = write!(said, " \u{b7} {}", fmt::count(t.lines().count(), "line", "lines"));
    }
    let base_head = match at {
        BlobAt::Rev(r) => crate::pagehead::file_head(r, &path, FileView::File, Tagline::of(&said)),
        BlobAt::Worktree => {
            said.push_str(" \u{b7} read from disk, not committed");
            crate::pagehead::worktree_file_head(&path, Tagline::of(&said))
        }
    };
    let page_head = PageHead {
        // Every file type keeps its path and tabs in view while the body scrolls. A rendered
        // body adds Preview/Code to the title row (a row of its own would stand alone); a
        // workbook has no text to switch to.
        pin: true,
        tools: {
            let switch = if (doc.is_some() || table.is_some()) && text.is_some() {
                crate::seg::Seg::preview_code().to_string()
            } else {
                String::new()
            };
            switch + &markdown::outline_menu(doc.as_ref().map_or(&symbols, |d| &d.outline), false)
        },
        ..base_head
    };
    let rendered = doc.map(|d| d.html).or(table);
    BlobPage {
        c: Chrome { md: rendered.is_some(), ..chrome },
        rendered,
        card_href: match at {
            BlobAt::Rev(r) => crate::card::href(r, &path),
            BlobAt::Worktree => String::new(),
        },
        outline_rail,
        page_head,
        size: fmt::size(b.size),
        raw_href,
        format: crate::filetype::describe(&path, &b.head),
        title: path,
        html,
        cut,
        cut_note: format!("Truncated after {} lines.", a.cfg().limits.blob_lines),
        media,
        archive,
        font,
        too_large,
    }
}

#[derive(Template)]
#[template(path = "blame.html")]
struct BlamePage {
    c: Chrome,
    title: String,
    head: PageHead,
    html: String,
}

/// Rows of blame per page and per fragment. About 400 bytes a row, so a page stays near
/// 1 MB however many lines the file has.
pub(crate) const BLAME_PAGE_LINES: usize = 2000;

/// `?from=` asks for the blame rows from that zero-based line on, without the page around them.
#[derive(serde::Deserialize)]
pub(super) struct BlameQuery {
    pub(super) from: Option<usize>,
}

/// Blame at one commit never changes, and its HTML is a row per line, so the page is rendered
/// once per (rev, commit, path), as the tree is.
pub(super) async fn blame(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
    Query(q): Query<BlameQuery>,
) -> Result<Html<String>> {
    let (rev_of, path_of) = (rev.clone(), path.clone());
    let (key, rev, path) = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let from = q.from.map_or_else(String::new, |f| f.to_string());
        let query = format!("{rev}\0{id}\0{}\0{from}", path.trim_matches('/'));
        Ok((a.memo_key("blame", Some(&query)), rev, path))
    })
    .await?;
    if let Some(h) = app.memo_get(&key) {
        return marked_blame(&app, &rev_of, &path_of, h).await;
    }
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let lines = a.repo.blame(&id, &path)?;
        let render = views::Render::of(a);
        let more = format!("/blame/{}/{}", fmt::enc_seg(&rev), fmt::enc_path(&path));
        if let Some(from) = q.from {
            return Ok(Err(views::blame_rows(
                &render,
                &path,
                &lines,
                from,
                fmt::now(),
                BLAME_PAGE_LINES,
                &fmt::href(&more),
            )));
        }
        // Blame is empty for binary and oversized files; say which, from the header size.
        // `raw_blob` told to skip reads the header only, not the content.
        let too_large = lines.is_empty()
            && a.repo.raw_blob(&id, &path, |_, _| true)?.size > a.repo.text_limit();
        let raw = crate::features::href_if(
            Feature::Raw,
            &format!("/raw/{}/{}", fmt::enc_seg(&rev), fmt::enc_path(&path)),
        );
        Ok(Ok(BlamePage {
            c: views::chrome_at(a, "files", &rev),
            head: PageHead {
                pin: true,
                ..crate::pagehead::file_head(
                    &rev,
                    &path,
                    FileView::Blame,
                    Tagline::of(&blame_said(&lines)),
                )
            },
            html: if too_large && raw.is_empty() {
                "<p class=\"muted pad\">File too large to blame.</p>".to_owned()
            } else if too_large {
                format!(
                    "<p class=\"muted pad\">File too large to blame. <a href=\"{}\">View \
                    raw</a>.</p>",
                    fmt::esc(&raw)
                )
            } else {
                views::blame_first(
                    &render,
                    &path,
                    &lines,
                    fmt::now(),
                    BLAME_PAGE_LINES,
                    &fmt::href(&more),
                )
            },
            title: path,
        }))
    })
    .await?;
    let html = match p {
        Ok(page) => page.render()?,
        Err(rows) => rows,
    };
    // The memo holds the page without marks: the basket changes without a commit.
    app.memo_put(key, &html);
    marked_blame(&app, &rev_of, &path_of, html).await
}

/// `html`, a blame page or its rows, with the basket's finding marks on the lines they point at.
async fn marked_blame(app: &Arc<App>, rev: &str, path: &str, html: String) -> Result<Html<String>> {
    let (rev, path) = (rev.to_owned(), path.trim_matches('/').to_owned());
    let html = blocking(app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let marks = crate::blob_marks::marks(a, Some(&id), &path);
        Ok(crate::blob_marks::apply_blame(html, &marks))
    })
    .await?;
    Ok(Html(html))
}

/// The blame tagline: how many lines, by how many authors, from how many commits.
fn blame_said(lines: &[gitterrace_git::BlameLine]) -> String {
    if lines.is_empty() {
        return "No lines to blame".to_owned();
    }
    let authors: std::collections::HashSet<&str> =
        lines.iter().map(|l| l.author.as_str()).collect();
    let commits: std::collections::HashSet<&str> =
        lines.iter().map(|l| l.commit.as_str()).collect();
    format!(
        "{} by {} from {}",
        fmt::count(lines.len(), "line", "lines"),
        fmt::count(authors.len(), "author", "authors"),
        fmt::count(commits.len(), "commit", "commits")
    )
}

/// Files blamed ahead of the first visitor after each index move.
const BLAME_WARM_FILES: usize = 20;
/// Longest the blame warm-up keeps starting files. It holds the warming guard, which defers the
/// idle release, and one large blame took 9 s on a Linux runner: 20 of them kept a server that
/// nobody browsed at its warm size for three minutes. Half the idle period bounds that.
const BLAME_WARM_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);

/// The files whose first blame costs most: line count times lines changed over history. That
/// tracks gix-blame's per-commit hunks x changes bookkeeping better than a commit count, and
/// the cache only holds a few large blames. Largest first.
pub(crate) fn blame_warm_list(
    files: &[gitterrace_index::HeadFile],
    src: &dyn InsightsSource,
    n: usize,
) -> Vec<String> {
    // Every commit, merges included: a merge's rows cost blame work too.
    let mut touches: std::collections::HashMap<gitterrace_index::feed::PathId, u64> =
        std::collections::HashMap::new();
    let all: Vec<Rank> = src.commits_in(CommitRange::All).map(|c| c.rank).collect();
    for (_, rows) in src.rows_for(all) {
        for f in rows {
            *touches.entry(f.path).or_default() += u64::from(f.add + f.del);
        }
    }
    let mut scored: Vec<(u64, &str)> = files
        .iter()
        .filter_map(|f| {
            let t = src.path_id(&f.path).and_then(|id| touches.get(&id).copied()).unwrap_or(0);
            (t > u64::from(f.lines) && f.lines > 0)
                .then_some((u64::from(f.lines) * t, f.path.as_str()))
        })
        .collect();
    scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(n).map(|(_, p)| p.to_owned()).collect()
}

/// Blames the costliest files at HEAD into the repository's bounded blame cache, one at a time
/// on one blocking thread and outside the request job slots, so it never makes a visitor wait.
/// Stops when HEAD moves or when the cache is three quarters full, leaving room for what
/// visitors open. A large translation catalog's blame is several MB.
/// An export run (the env with a change memo) skips it: it writes no blame page, and gix holds
/// every past version of a blamed file while it runs, about 1 GB for one rust file.
pub(crate) async fn prewarm_blame(app: Arc<App>) {
    prewarm_blame_within(app, BLAME_WARM_BUDGET).await;
}

/// [`prewarm_blame`], starting no file once `budget` has passed.
pub(crate) async fn prewarm_blame_within(app: Arc<App>, budget: std::time::Duration) {
    if app.env.changes.is_some() {
        return;
    }
    let work = tokio::task::spawn_blocking(move || {
        let end = std::time::Instant::now() + budget;
        let Ok(head) = app.repo.resolve("HEAD") else {
            return;
        };
        let Ok(Some(src)) = app.source(&head) else {
            return;
        };
        let files = app.head_lines(&head);
        // A list built while the cache failed a read is dropped: prewarming is best effort.
        let Ok(list) = src.run(|s| blame_warm_list(&files, &**s, BLAME_WARM_FILES)) else {
            return;
        };
        for path in list {
            let (used, limit) = app.repo.blame_cache_fill();
            if std::time::Instant::now() >= end
                || used * 4 >= limit * 3
                || app.repo.resolve("HEAD").ok().as_ref() != Some(&head)
            {
                return;
            }
            if app.repo.blame_is_cached(&head, &path) {
                continue;
            }
            // A failed blame only means the first visitor computes it.
            if let Err(e) = app.repo.blame(&head, &path) {
                tracing::debug!("blame prewarm of {path}: {e}");
            }
        }
    });
    if let Err(e) = work.await {
        tracing::debug!("blame prewarm stopped: {e}");
    }
}
