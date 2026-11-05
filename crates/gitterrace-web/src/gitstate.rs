//! Pages for state that lives in the git directory: linked worktrees, stashes and annotated
//! tags. The branch table on `/refs` uses the same reads.

use askama::Template;
use axum::{
    extract::{Path, State},
    response::Html,
};

use crate::{
    app::blocking,
    empty::Empty,
    error::Result,
    fmt::{self, ago, enc_seg, href, sha_link},
    live::Mode,
    routes::Shared,
    views::{self, Chrome},
};

struct WorktreeRow {
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
    reftabs: Vec<crate::views::RefTab>,
    rows: Vec<WorktreeRow>,
    empty: Option<Empty>,
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
                        href(&format!("/log/{}", enc_seg(&target)))
                    },
                    branch: w.branch.unwrap_or_else(|| "detached".to_owned()),
                    commit: w.commit.as_deref().map(sha_link).unwrap_or_default(),
                    path: w.path.unwrap_or_default(),
                    id: w.id,
                    locked: w.locked,
                }
            })
            .collect::<Vec<_>>();
        Ok(WorktreesPage {
            empty: rows
                .is_empty()
                .then(|| Empty::new("No linked worktrees.").step("Branches & tags", href("/refs"))),
            c: views::chrome(a, "refs").live(Mode::WORKTREES),
            reftabs: views::ref_tabs(a, "worktrees")?,
            rows,
        }
        .render()?)
    })
    .await?;
    Ok(Html(html))
}

struct StashRow {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) message: String,
    pub(crate) when: crate::when::When,
}

#[derive(Template)]
#[template(path = "stashes.html")]
struct StashesPage {
    c: Chrome,
    reftabs: Vec<crate::views::RefTab>,
    rows: Vec<StashRow>,
    empty: Option<Empty>,
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
            .collect::<Vec<_>>();
        Ok(StashesPage {
            empty: rows
                .is_empty()
                .then(|| Empty::new("No stashes.").step("Branches & tags", href("/refs"))),
            c: views::chrome(a, "refs").live(Mode::STASHES),
            reftabs: views::ref_tabs(a, "stashes")?,
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
pub(crate) fn branch_rows(a: &crate::app::App) -> Result<Vec<views::CommitRow>> {
    let default = a.repo.head_branch();
    let now = fmt::now();
    let base = default.clone().unwrap_or_else(|| "HEAD".to_owned());
    let status = a.repo.branch_status(&base)?;
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
