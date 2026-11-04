//! The tree, blob and blame pages, and the blame warm-up.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use gitterrace_index::feed::{CommitRange, InsightsSource, Rank};

use crate::routes::{DocQuery, MAX_PREVIEW_BYTES, Shared, page};
use crate::{
    app::{App, blocking},
    error::Result,
    fmt, markdown, views,
    views::{Chrome, Crumb, Entry},
};

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

pub(super) async fn tree_root(
    state: Shared,
    Path(rev): Path<String>,
    q: Query<DocQuery>,
) -> Result<Html<String>> {
    tree(state, Path((rev, String::new())), q).await
}

pub(super) async fn tree(
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
    outline_rail: String,
    outline_menu: String,
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
        Ok(views::code_tail_html(a.code_cache_bytes(), &path, &text))
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
        let doc = text.filter(|_| is_markdown(&path)).map(|t| {
            let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
            markdown::render_doc(t, markdown::Base::new(&rev, dir))
        });
        let outline_rail = doc
            .as_ref()
            .map_or_else(String::new, |d| markdown::outline_rail(&d.outline));
        let outline_menu = doc
            .as_ref()
            .map_or_else(String::new, |d| markdown::outline_menu(&d.outline, false));
        let rendered = doc.map(|d| d.html);
        let (r, pe) = (fmt::enc_seg(&rev), fmt::enc_path(&path));
        Ok(BlobPage {
            c: Chrome {
                md: rendered.is_some(),
                ..views::chrome_at(a, "files", &rev)
            },
            rendered,
            outline_rail,
            outline_menu,
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

pub(super) async fn blame(
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
                .find_path(&f.path)
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
        for path in blame_warm_list(&files, &*src, BLAME_WARM_FILES) {
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
