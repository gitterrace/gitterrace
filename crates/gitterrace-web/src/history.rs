//! The file history page, `/history/{path}`: every commit that touched one file.

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Path, RawQuery, State};
use axum::response::Html;

use crate::{
    app::App,
    error::Result,
    fmt::{self, href},
    insights::{cached_keyed, insights},
    routes::Shared,
    views::{self, Chrome},
};

#[derive(Template)]
#[template(path = "history.html")]
struct HistoryPage {
    c: Chrome,
    kpis: Vec<crate::insights::Kpi>,
    path: String,
    head: crate::pagehead::PageHead,
    rows: Vec<views::CommitRow>,
    empty: Option<crate::empty::Empty>,
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
        let subjects: HashMap<String, String> = match a.repo.raw_summaries(&ids) {
            Ok(s) => s.into_iter().map(|s| (s.id, s.subject)).collect(),
            Err(e) => {
                tracing::warn!("file history: subjects unavailable: {e}");
                HashMap::new()
            }
        };
        // One lookup per listed commit, never a pass over the history.
        let emails: HashMap<&str, String> = ids
            .iter()
            .filter_map(|id| {
                let c = r.commit(r.commit_of(id)?)?;
                Some((id.as_str(), r.ident(c.ident)?.1.to_string()))
            })
            .collect();
        let kpis = history_kpis(&touches);
        let rows: Vec<views::CommitRow> = touches
            .into_iter()
            .map(|t| {
                let email = emails.get(t.commit.as_str()).map_or("", String::as_str);
                let mut note = format!("{} changed", fmt::count(t.lines, "line", "lines"));
                if t.path != path {
                    note.push_str(" · as ");
                    note.push_str(&t.path);
                }
                views::CommitRow {
                    href: fmt::commit_file_href(&t.commit, &t.path),
                    sha: fmt::sha_link(&t.commit),
                    initial: views::initials(&t.author),
                    hue: views::hue(&t.author),
                    subject: subjects.get(&t.commit).cloned().unwrap_or_default(),
                    author_href: views::author_href(email),
                    author: t.author,
                    when: fmt::ago(t.time, now),
                    ..views::CommitRow::default()
                }
                .note(note)
            })
            .collect();
        HistoryPage {
            kpis,
            // Built through the insights helper, but one file's history is a code view.
            c: c.active("files"),
            head: crate::pagehead::PageHead {
                pin: true,
                ..crate::pagehead::file_head(
                    &fmt::selected(),
                    &path,
                    crate::pagehead::FileView::History,
                    crate::pagehead::Tagline::of(&format!(
                        "{}, following renames",
                        fmt::count(rows.len(), "commit", "commits")
                    )),
                )
            },
            empty: rows.is_empty().then(|| {
                crate::empty::Empty::new("No history found for this path.")
                    .step("Browse the files", href(&format!("/tree/{}", fmt::selected_seg())))
            }),
            path,
            rows,
        }
    })
    .await
}

/// The big numbers of one file's history, from the touches the page lists: commits, authors,
/// lines changed and the day it was first changed.
fn history_kpis(touches: &[gitterrace_index::Touch]) -> Vec<crate::insights::Kpi> {
    use crate::insights::Kpi;
    let authors: std::collections::HashSet<&str> =
        touches.iter().map(|t| t.author.as_str()).collect();
    let lines: u64 = touches.iter().map(|t| u64::from(t.lines)).sum();
    let first = touches.iter().map(|t| t.time).min();
    vec![
        Kpi("Commits", fmt::thousands(touches.len())),
        Kpi("Authors", fmt::thousands(authors.len())),
        Kpi("Lines changed", fmt::thousands(lines)),
        Kpi("First changed", first.map_or_else(|| "-".to_owned(), fmt::day)),
    ]
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
        for uri in ["/log/HEAD?q=path:sub%2Ff", "/log/HEAD?q=path:sub%2Ff&page=1", "/history/sub/f"]
        {
            let (status, _) = gitterrace_testkit::get(&router, uri).await;
            assert_eq!(status, 200, "{uri}");
        }
    }

    /// Each commit link of a file's history opens the commit on that file's section.
    #[tokio::test]
    async fn commit_links_carry_the_file_fragment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = crate::wiring_tests::repo(dir.path(), "r");
        let app = crate::wiring_tests::app(&p, gitterrace_config::Config::default());
        app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
        app.mark_ready();
        let router = routes::router(app);
        let (_, page) = gitterrace_testkit::get(&router, "/history/small.txt").await;
        let links = page.matches("/commit/").count();
        let anchored = page.matches("#p/small.txt\"").count();
        assert!(links >= 2 && anchored >= 2, "{links} links, {anchored} anchored");
    }
}
