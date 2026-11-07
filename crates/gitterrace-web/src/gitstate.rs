//! Pages for state that lives in the git directory: linked worktrees, stashes and annotated
//! tags. The branch table on `/refs` uses the same reads.

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, Redirect},
};

use crate::{
    app::blocking,
    error::Result,
    fmt::{self, ago, enc_seg, href, sha_link},
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

/// Linked worktrees and stashes are this machine's state, not history: never on a public
/// server, and only with the worktree feature on.
pub(crate) fn local_state(a: &crate::app::App) -> bool {
    !a.public() && crate::features::on(gitterrace_config::Feature::Worktree)
}

pub(crate) fn worktree_rows(a: &crate::app::App) -> Result<Vec<WorktreeRow>> {
    Ok(a.repo
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
                    href(&format!("/log/{}", enc_seg(&target)))
                },
                branch: w.branch.unwrap_or_else(|| "detached".to_owned()),
                commit: w.commit.as_deref().map(sha_link).unwrap_or_default(),
                path: w.path.unwrap_or_default(),
                id: w.id,
                locked: w.locked,
            }
        })
        .collect())
}

pub(crate) struct StashRow {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) message: String,
    pub(crate) when: crate::when::When,
}

pub(crate) fn stash_rows(a: &crate::app::App) -> Result<Vec<StashRow>> {
    let now = fmt::now();
    Ok(a.repo
        .stashes()?
        .into_iter()
        .map(|s| StashRow {
            name: format!("stash@{{{}}}", s.index),
            href: href(&format!("/commit/{}", s.commit)),
            message: s.message,
            when: ago(s.time, now),
        })
        .collect())
}

/// The old pages for worktrees and stashes, now sections of `/refs`.
pub(crate) async fn worktrees(State(app): Shared) -> Redirect {
    Redirect::permanent(&format!("{}/refs#worktrees", app.base))
}

pub(crate) async fn stashes(State(app): Shared) -> Redirect {
    Redirect::permanent(&format!("{}/refs#stashes", app.base))
}

#[derive(Template)]
#[template(path = "tag.html")]
struct TagPage {
    c: Chrome,
    name: String,
    /// The tagged commit, as every list shows a commit.
    commit: views::CommitRow,
    tree_href: String,
    tagger: String,
    when: crate::when::When,
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
            c: views::chrome(a, "refs").pinned(),
            commit: a
                .repo
                .summaries(std::slice::from_ref(&r.target))?
                .first()
                .map(|s| views::CommitRow::of(s, now))
                .unwrap_or_default(),
            tree_href: href(&format!("/tree/{}", enc_seg(&r.name))),
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

/// Branch rows on `/refs`, against HEAD's branch (or HEAD when detached), default first and
/// then newest commit first: the one commit row, led by the branch, closed by its divergence.
///
/// Only the first `limit` rows in that order are built: `refs` comes newest first, so they
/// are picked before any commit is read or any divergence walked.
pub(crate) fn branch_rows(
    a: &crate::app::App,
    refs: &[gitterrace_git::RefInfo],
    limit: usize,
) -> Result<Vec<views::CommitRow>> {
    let default = a.repo.head_branch();
    let now = fmt::now();
    let base = default.clone().unwrap_or_else(|| "HEAD".to_owned());
    let branches = refs
        .iter()
        .filter(|r| r.kind == gitterrace_git::RefKind::Branch);
    let is_default = |r: &&gitterrace_git::RefInfo| default.as_deref() == Some(r.name.as_str());
    let picked = branches
        .clone()
        .filter(is_default)
        .chain(branches.filter(|r| !is_default(r)))
        .take(limit)
        .cloned();
    let status = a.repo.branch_status_of(&base, picked)?;
    let ids: Vec<String> = status.iter().map(|s| s.target.clone()).collect();
    let summaries = a.repo.summaries(&ids)?;
    let mut rows: Vec<(bool, i64, views::CommitRow)> = status
        .into_iter()
        .zip(summaries)
        .map(|(s, c)| {
            let is_default = default.as_deref() == Some(s.name.as_str());
            let mut badges = Vec::new();
            if is_default {
                badges.push("default");
            }
            // Merged means "in the default branch"; a detached HEAD is no such branch.
            if s.merged() && !is_default && default.is_some() {
                badges.push("merged");
            }
            if !is_default && fmt::selected() == s.name {
                badges.push("selected");
            }
            let mut row = views::CommitRow::of(&c, now).led_by(views::Lead {
                href: href(&format!("/log/{}", enc_seg(&s.name))),
                name: s.name.clone(),
                badges,
            });
            // The default branch is the baseline: it is never ahead of or behind itself.
            if !is_default {
                row.counts = vec![
                    views::Count {
                        label: "behind",
                        value: fmt::thousands(s.behind),
                        href: String::new(),
                        tip: "Commits behind the default branch",
                    },
                    views::Count {
                        label: "ahead",
                        value: fmt::thousands(s.ahead),
                        href: crate::compare::compare_href(Some(&base), &s.name),
                        tip: "Commits ahead of the default branch",
                    },
                ];
            }
            (is_default, c.committer.time, row)
        })
        .collect();
    rows.sort_by_key(|(d, t, _)| (!*d, std::cmp::Reverse(*t)));
    Ok(rows.into_iter().map(|(_, _, r)| r).collect())
}
