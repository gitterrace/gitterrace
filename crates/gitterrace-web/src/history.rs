//! The file history page, `/history/{path}`: every commit that touched one file.

use std::collections::HashMap;

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
    when: crate::when::When,
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
        let now = fmt::now();
        let touches = ins.file_history(&path);
        // Only the listed commits are read: a subject per row, not a scan of all history, and
        // one id map instead of a search of every record per row.
        let ids: Vec<String> = touches.iter().map(|t| t.commit.clone()).collect();
        let subjects: HashMap<String, String> = a
            .repo
            .raw_summaries(&ids)
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.id, s.subject))
            .collect();
        // One lookup per listed commit, never a pass over the history.
        let emails: HashMap<&str, String> = ids
            .iter()
            .filter_map(|id| {
                let c = r.commit(r.find_commit(id)?)?;
                Some((id.as_str(), r.ident(c.ident)?.1.to_string()))
            })
            .collect();
        let rows: Vec<HistRow> = touches
            .into_iter()
            .map(|t| {
                let email = emails.get(t.commit.as_str()).map_or("", String::as_str);
                HistRow {
                    href: href(&format!("/commit/{}", t.commit)),
                    sha: fmt::sha_link(&t.commit),
                    subject: subjects.get(&t.commit).cloned().unwrap_or_default(),
                    author_href: views::author_href(email),
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
            blame_href: crate::features::href_if(
                gitterrace_config::Feature::Blame,
                &format!("/blame/{}/{}", fmt::selected_seg(), enc_path(&path)),
            ),
            path,
            total: fmt::count(rows.len(), "commit", "commits"),
            rows,
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gitterrace_testkit::{git, git_at};

    use crate::{app::App, routes};

    /// A directory that was a submodule at an older commit (rust's `library/stdarch`): every
    /// page that looks a path up there answers cleanly, never with a 500 from reading the
    /// other repository's commit as a tree.
    #[tokio::test]
    async fn paths_through_a_former_submodule_never_fail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        let d = "2024-01-01T00:00:00+00:00";
        git_at(p, d, &["init", "-q", "-b", "main"]);
        let info = "160000,1dfaa4db2479753a46a3e90f2c3c89d89d0b21f1,sub";
        git_at(p, d, &["update-index", "--add", "--cacheinfo", info]);
        git_at(p, d, &["commit", "-q", "-m", "submodule"]);
        let old = git(p, &["rev-parse", "HEAD"]);
        git_at(p, d, &["rm", "-q", "--cached", "sub"]);
        std::fs::create_dir_all(p.join("sub")).expect("mkdir");
        std::fs::write(p.join("sub/f"), "x\n").expect("write");
        git_at(p, d, &["add", "sub"]);
        git_at(p, d, &["commit", "-q", "-m", "vendored"]);
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        let router = routes::router(Arc::clone(&app));
        for page in ["blob", "tree", "raw", "blame", "hl"] {
            for path in ["sub/f", "sub"] {
                let uri = format!("/{page}/{old}/{path}");
                let (status, _) = gitterrace_testkit::get(&router, &uri).await;
                assert!(status < 500, "{uri} answered {status}");
            }
        }
        for uri in [
            "/log/HEAD?path=sub/f",
            "/log/HEAD?page=1&path=sub/f",
            "/history/sub/f",
        ] {
            let (status, _) = gitterrace_testkit::get(&router, uri).await;
            assert_eq!(status, 200, "{uri}");
        }
    }
}
