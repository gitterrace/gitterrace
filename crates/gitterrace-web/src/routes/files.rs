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

use crate::dochead::DocHead;
use crate::pagehead::{Action, FileView, PageHead, Tagline};
use crate::routes::{DocQuery, Shared, page};
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
}

pub(super) async fn tree_root(
    state: Shared,
    Path(rev): Path<String>,
    q: Query<DocQuery>,
) -> Result<Html<String>> {
    tree(state, Path((rev, String::new())), q).await
}

/// A tree at one commit never changes, so its page (README card included) is rendered once per
/// (rev, commit, path, doc). The memo key still moves with HEAD, the index and config.
pub(super) async fn tree(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
    Query(q): Query<DocQuery>,
) -> Result<Html<String>> {
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
        return Ok(Html(h));
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
            about,
            title: if path.is_empty() {
                rev.clone()
            } else {
                path.clone()
            },
            head: tree_head(&rev, &path, &list),
            entries: views::entries(&rev, &path, &list),
            path_enc: fmt::enc_path(&path),
            up,
        })
    })
    .await?;
    let html = p.render()?;
    app.memo_put(key, Arc::new(html.clone()));
    Ok(Html(html))
}

/// A folder's page head: its path as the title, what it holds as the tagline, and its history
/// as the action.
fn tree_head(rev: &str, path: &str, list: &[gitterrace_git::TreeEntry]) -> PageHead {
    let dirs = list
        .iter()
        .filter(|e| e.kind == gitterrace_git::TreeEntryKind::Dir)
        .count();
    let files = list.len() - dirs;
    let said = match (dirs, files) {
        (0, 0) => "An empty folder".to_owned(),
        (0, f) => fmt::count(f, "file", "files"),
        (d, 0) => fmt::count(d, "folder", "folders"),
        (d, f) => format!(
            "{} and {}",
            fmt::count(d, "folder", "folders"),
            fmt::count(f, "file", "files")
        ),
    };
    let r = fmt::enc_seg(rev);
    PageHead {
        crumbs: views::crumbs(rev, path, false),
        actions: vec![Action {
            label: "History",
            href: fmt::href(&format!("/log/{r}?path={}", fmt::enc_path(path))),
        }],
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
    too_large: bool,
    raw_href: String,
    rendered: Option<String>,
    /// "On this page" as the rail: the document's headings, or a source file's symbols.
    outline_rail: String,
    /// The header of the Markdown card; `None` for any other file.
    head: Option<DocHead>,
}

fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".md", ".markdown", ".mdown"]
        .iter()
        .any(|e| lower.ends_with(e))
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
) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let kind = crate::media::kind(&path);
        // One content read serves the text and, for media small enough to want facts about,
        // the bytes.
        let (b, bytes) = if kind.is_some() {
            a.repo
                .blob_with_bytes(&id, &path, a.cfg().limits.preview_mb << 20)?
        } else {
            (a.repo.blob(&id, &path)?, Vec::new())
        };
        let media = match kind {
            Some(k) => {
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
            let (h, cut) = views::code_html_first(&views::Render::of(a), &path, t);
            (Some(h), cut)
        });
        let preview = crate::features::on(Feature::MarkdownPreview);
        let doc = text.filter(|_| preview && is_markdown(&path)).map(|t| {
            let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
            markdown::render_doc(
                t,
                markdown::Base::new(&rev, dir).highlight(views::Render::of(a).highlight_bytes),
            )
        });
        let symbols = match (&doc, text, media.is_some()) {
            (None, Some(t), false) => {
                crate::symbols::outline(&path, t, views::Render::of(a).blob_lines)
            }
            _ => Vec::new(),
        };
        let outline_rail = doc.as_ref().map_or_else(
            || markdown::outline_rail(&symbols),
            |d| markdown::outline_rail(&d.outline),
        );
        let (r, pe) = (fmt::enc_seg(&rev), fmt::enc_path(&path));
        let mut said = fmt::size(b.size);
        if let Some(t) = text {
            let _ = write!(
                said,
                " \u{b7} {}",
                fmt::count(t.lines().count(), "line", "lines")
            );
        }
        let page_head = PageHead {
            // A source file's path stays in view while its lines scroll; a document's own card
            // head pins instead.
            pin: doc.is_none() && media.is_none(),
            tools: if doc.is_none() {
                markdown::outline_menu(&symbols, false)
            } else {
                String::new()
            },
            ..crate::pagehead::file_head(&rev, &path, FileView::File, Tagline::of(&said))
        };
        let head = doc.as_ref().map(|d| DocHead {
            title: path.rsplit('/').next().unwrap_or(&path).to_owned(),
            views: true,
            outline: markdown::outline_menu(&d.outline, true),
            ..DocHead::default()
        });
        let rendered = doc.map(|d| d.html);
        Ok(BlobPage {
            c: Chrome {
                md: rendered.is_some(),
                ..views::chrome_at(a, "files", &rev)
            },
            rendered,
            outline_rail,
            head,
            page_head,
            size: fmt::size(b.size),
            raw_href: crate::features::href_if(Feature::Raw, &format!("/raw/{r}/{pe}")),
            title: path,
            html,
            cut,
            cut_note: format!("Truncated after {} lines.", a.cfg().limits.blob_lines),
            media,
            too_large,
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
    head: PageHead,
    html: String,
}

pub(super) async fn blame(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Html<String>> {
    let p = blocking(&app, move |a| {
        let id = a.repo.resolve(&rev)?;
        let path = path.trim_matches('/').to_owned();
        let lines = a.repo.blame(&id, &path)?;
        // Blame is empty for binary and oversized files; say which, from the header size.
        // `raw_blob` told to skip reads the header only, not the content.
        let too_large = lines.is_empty()
            && a.repo.raw_blob(&id, &path, |_, _| true)?.size > a.repo.text_limit();
        let raw = crate::features::href_if(
            Feature::Raw,
            &format!("/raw/{}/{}", fmt::enc_seg(&rev), fmt::enc_path(&path)),
        );
        Ok(BlamePage {
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
                views::blame_html(&views::Render::of(a), &path, &lines, fmt::now())
            },
            title: path,
        })
    })
    .await?;
    page(&p)
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
            let t = src
                .path_id(&f.path)
                .and_then(|id| touches.get(&id).copied())
                .unwrap_or(0);
            (t > u64::from(f.lines) && f.lines > 0)
                .then_some((u64::from(f.lines) * t, f.path.as_str()))
        })
        .collect();
    scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(n)
        .map(|(_, p)| p.to_owned())
        .collect()
}

/// Blames the costliest files at HEAD into the repository's bounded blame cache, one at a time
/// on one blocking thread and outside the request job slots, so it never makes a visitor wait.
/// Stops when HEAD moves or when the cache is three quarters full, leaving room for what
/// visitors open. A large translation catalog's blame is several MB.
pub(crate) async fn prewarm_blame(app: Arc<App>) {
    let work = tokio::task::spawn_blocking(move || {
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
            if used * 4 >= limit * 3 || app.repo.resolve("HEAD").ok().as_ref() != Some(&head) {
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
