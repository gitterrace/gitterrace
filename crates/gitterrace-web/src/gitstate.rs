//! Pages for state that lives in the git directory: linked worktrees, stashes and annotated
//! tags. The branch table on `/refs` uses the same reads.

use askama::Template;
use axum::{
    extract::{Path, State},
    response::Html,
};

use crate::{
    app::blocking,
    error::Result,
    fmt::{self, ago, enc, href, short},
    routes::Shared,
    views::{self, Chrome},
};

pub(crate) struct WorktreeRow {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) branch: String,
    pub(crate) log: String,
    pub(crate) commit: String,
    pub(crate) locked: bool,
}

#[derive(Template)]
#[template(path = "worktrees.html")]
struct WorktreesPage {
    c: Chrome,
    rows: Vec<WorktreeRow>,
}

pub(crate) async fn worktrees(State(app): Shared) -> Result<Html<String>> {
    let html = blocking(&app, |a| {
        let rows = a
            .repo
            .linked_worktrees()?
            .into_iter()
            .map(|w| {
                let target = w
                    .branch
                    .clone()
                    .or_else(|| w.commit.clone())
                    .unwrap_or_default();
                WorktreeRow {
                    log: if target.is_empty() {
                        String::new()
                    } else {
                        href(&format!("/log/{}", enc(&target, false)))
                    },
                    branch: w.branch.unwrap_or_else(|| "detached".to_owned()),
                    commit: w.commit.as_deref().map(short).unwrap_or_default(),
                    path: w.path.unwrap_or_default(),
                    id: w.id,
                    locked: w.locked,
                }
            })
            .collect();
        Ok(WorktreesPage {
            c: views::chrome(a, "refs"),
            rows,
        }
        .render()?)
    })
    .await?;
    Ok(Html(html))
}

pub(crate) struct StashRow {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) message: String,
    pub(crate) when: String,
}

#[derive(Template)]
#[template(path = "stashes.html")]
struct StashesPage {
    c: Chrome,
    rows: Vec<StashRow>,
}

pub(crate) async fn stashes(State(app): Shared) -> Result<Html<String>> {
    let html = blocking(&app, |a| {
        let now = fmt::now();
        let rows = a
            .repo
            .stashes()?
            .into_iter()
            .map(|s| StashRow {
                name: format!("stash@{{{}}}", s.index),
                href: href(&format!("/commit/{}", s.commit)),
                message: s.message,
                when: ago(s.time, now),
            })
            .collect();
        Ok(StashesPage {
            c: views::chrome(a, "refs"),
            rows,
        }
        .render()?)
    })
    .await?;
    Ok(Html(html))
}

#[derive(Template)]
#[template(path = "tag.html")]
struct TagPage {
    c: Chrome,
    name: String,
    commit: String,
    commit_href: String,
    tree_href: String,
    tagger: String,
    when: String,
    subject: String,
    body: String,
}

pub(crate) async fn tag(State(app): Shared, Path(name): Path<String>) -> Result<Html<String>> {
    let html = blocking(&app, move |a| {
        let r = a
            .repo
            .refs()?
            .into_iter()
            .find(|r| r.kind == gitterrace_git::RefKind::Tag && r.name == name)
            .ok_or_else(|| gitterrace_git::Error::RevNotFound(name.clone()))?;
        let now = fmt::now();
        let note = r.annotation.as_ref();
        Ok(TagPage {
            c: views::chrome(a, "refs"),
            commit: short(&r.target),
            commit_href: href(&format!("/commit/{}", r.target)),
            tree_href: href(&format!("/tree/{}", enc(&r.name, false))),
            tagger: note
                .and_then(|n| n.tagger.as_ref())
                .map(|t| t.name.clone())
                .unwrap_or_default(),
            when: note
                .and_then(|n| n.tagger.as_ref())
                .map_or_else(|| ago(r.time, now), |t| ago(t.time, now)),
            subject: note.map(|n| n.subject.clone()).unwrap_or_default(),
            body: note.map(|n| n.body.clone()).unwrap_or_default(),
            name: r.name,
        }
        .render()?)
    })
    .await?;
    Ok(Html(html))
}

/// A local branch row on `/refs`: divergence from the default branch and its last commit.
pub(crate) struct BranchRow {
    pub(crate) name: String,
    pub(crate) log: String,
    pub(crate) compare: String,
    pub(crate) commit: String,
    pub(crate) subject: String,
    pub(crate) author: String,
    pub(crate) when: String,
    pub(crate) ahead: String,
    pub(crate) behind: String,
    pub(crate) merged: bool,
    pub(crate) default: bool,
}

/// Branch rows against HEAD's branch (or HEAD when detached), newest commit first.
pub(crate) fn branch_rows(a: &crate::app::App) -> Result<Vec<BranchRow>> {
    let default = a.repo.head_branch();
    let now = fmt::now();
    let base = default.clone().unwrap_or_else(|| "HEAD".to_owned());
    let status = a.repo.branch_status(&base)?;
    let ids: Vec<String> = status.iter().map(|s| s.target.clone()).collect();
    let summaries = a.repo.summaries(&ids)?;
    let mut rows: Vec<(i64, BranchRow)> = status
        .into_iter()
        .zip(summaries)
        .map(|(s, c)| {
            let is_default = default.as_deref() == Some(s.name.as_str());
            let row = BranchRow {
                log: href(&format!("/log/{}", enc(&s.name, false))),
                compare: href(&format!(
                    "/compare?base={}&head={}",
                    enc(&base, false),
                    enc(&s.name, false)
                )),
                commit: short(&s.target),
                subject: c.subject,
                author: c.author.name,
                when: ago(c.committer.time, now),
                ahead: fmt::thousands(s.ahead),
                behind: fmt::thousands(s.behind),
                // Merged means "in the default branch"; a detached HEAD is no such branch.
                merged: s.merged() && !is_default && default.is_some(),
                default: is_default,
                name: s.name,
            };
            (c.committer.time, row)
        })
        .collect();
    rows.sort_by_key(|(t, r)| (!r.default, std::cmp::Reverse(*t)));
    Ok(rows.into_iter().map(|(_, r)| r).collect())
}
