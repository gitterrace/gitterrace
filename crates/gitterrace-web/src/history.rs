//! The file history page, `/history/{path}`: every commit that touched one file.

use askama::Template;
use axum::extract::{Path, RawQuery, State};
use axum::response::Html;

use crate::{
    app::App,
    error::Result,
    fmt::{self, enc_path, href},
    insights::{cached_keyed, insights},
    routes::Shared,
    views::{self, Chrome},
};

struct HistRow {
    href: String,
    sha: String,
    subject: String,
    author: String,
    author_href: String,
    when: String,
    lines: String,
    renamed: Option<String>,
}

#[derive(Template)]
#[template(path = "history.html")]
struct HistoryPage {
    c: Chrome,
    path: String,
    blob_href: String,
    blame_href: String,
    total: String,
    rows: Vec<HistRow>,
}

pub(crate) async fn page(
    State(app): Shared,
    Path(path): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Html<String>> {
    let path = path.trim_matches('/').to_owned();
    let key = format!("{path}\0{}", App::canonical_query(raw.as_deref()));
    cached_keyed(&app, "hi", key, move |a, r, c| {
        let ins = insights(a, r, a.defaults().into_options());
        let head = a.repo.resolve(&fmt::selected()).unwrap_or_default();
        let subjects = a.subjects(&head);
        let now = fmt::now();
        let rows: Vec<HistRow> = ins
            .file_history(&path)
            .into_iter()
            .map(|t| {
                let email = r
                    .iter()
                    .find(|x| x.id == t.commit)
                    .map_or_else(String::new, |x| x.email.clone());
                HistRow {
                    href: href(&format!("/commit/{}", t.commit)),
                    sha: fmt::sha_link(&t.commit),
                    subject: subjects.get(&t.commit).cloned().unwrap_or_default(),
                    author_href: views::author_href(&email),
                    author: t.author,
                    when: fmt::ago(t.time, now),
                    lines: fmt::count(t.lines, "line", "lines"),
                    renamed: (t.path != path).then_some(t.path),
                }
            })
            .collect();
        HistoryPage {
            c,
            blob_href: href(&format!(
                "/blob/{}/{}",
                fmt::selected_seg(),
                enc_path(&path)
            )),
            blame_href: href(&format!(
                "/blame/{}/{}",
                fmt::selected_seg(),
                enc_path(&path)
            )),
            path,
            total: fmt::count(rows.len(), "commit", "commits"),
            rows,
        }
    })
    .await
}
