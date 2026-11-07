//! `/`: the repositories home, each row leading with what changed today.

use std::fmt::Write as _;
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use gitterrace_git::CommitSummary;

use crate::fmt;
use crate::multi::Table;

#[derive(Template)]
#[template(path = "repos.html")]
struct IndexPage {
    rows: Vec<IndexRow>,
    /// The card's header: "Repositories" and "2 repositories, 14 commits today".
    head: crate::dochead::DocHead,
}

struct IndexRow {
    name: String,
    href: String,
    when: crate::when::When,
    commit_href: String,
    spark: String,
    /// Commits in the sparkline's window.
    total: u32,
    tip: String,
    /// What changed today, the line a returning reader looks for first.
    today: String,
    commits_today: u32,
    /// Uncommitted work in the working tree; `None` where the tree is not shown.
    dirty: Option<bool>,
}

/// Bucket counts as a polyline across a 90x20 box, scaled to `max`.
fn polyline(counts: &[u32], max: u32) -> String {
    let max = max.max(1);
    let one = [counts.first().copied().unwrap_or(0); 2];
    let counts = if counts.len() < 2 { &one[..] } else { counts };
    let mut pts = String::new();
    for (i, n) in counts.iter().enumerate() {
        let x = i as f64 * 90.0 / (counts.len() - 1) as f64;
        let y = 19.0 - f64::from(*n) * 18.0 / f64::from(max);
        let _ = write!(pts, "{x:.1},{y:.1} ");
    }
    pts.trim_end().to_owned()
}

/// The row's story of today: how many commits landed and the newest one's subject, or how long
/// it has been quiet. Read before the subject on purpose: "3 commits today" is the news.
fn today_line(today: u32, last: Option<&CommitSummary>, now: i64) -> String {
    let Some(c) = last else {
        return "No commits yet".to_owned();
    };
    if today > 0 {
        format!(
            "{} today, last: {}",
            fmt::count(today, "commit", "commits"),
            c.subject
        )
    } else {
        format!(
            "Quiet today. Last commit {}: {}",
            fmt::ago(c.author.time, now).text,
            c.subject
        )
    }
}

/// `/`: the one repository while there is one (a 302, so the answer changes once another
/// joins), else the index of all.
pub(super) async fn index(State(table): State<Arc<Table>>) -> crate::error::Result<Response> {
    if let [one] = table.snapshot().as_slice() {
        let to = format!("/r/{}/", one.name);
        return Ok((StatusCode::FOUND, [(axum::http::header::LOCATION, to)]).into_response());
    }
    let html = tokio::task::spawn_blocking(move || {
        let now = fmt::now();
        let show_tree =
            !table.env.public() && crate::features::on(gitterrace_config::Feature::Worktree);
        let rows: Vec<IndexRow> = table
            .snapshot()
            .iter()
            .map(|s| {
                let href = format!("/r/{}/", s.name);
                let r = s.recent(now);
                let c = r.last.as_ref();
                let counts = &r.series.counts;
                let peak = counts.iter().copied().max().unwrap_or(0);
                let total: u32 = counts.iter().sum();
                IndexRow {
                    name: s.name.clone(),
                    when: c.map_or_else(crate::when::When::none, |c| fmt::ago(c.author.time, now)),
                    commit_href: c.map_or_else(
                        || href.clone(),
                        |c| format!("/r/{}/commit/{}", s.name, c.id),
                    ),
                    spark: polyline(counts, peak),
                    total,
                    today: today_line(r.today, c, now),
                    commits_today: r.today,
                    dirty: show_tree.then(|| s.dirty()),
                    tip: if counts.is_empty() {
                        "No commits yet.".to_owned()
                    } else {
                        format!(
                            "{}: {}, busiest {peak}.",
                            r.series.label(),
                            fmt::count(total, "commit", "commits")
                        )
                    },
                    href,
                }
            })
            .collect();
        let sub = format!(
            "{}, {} today",
            fmt::count(rows.len(), "repository", "repositories"),
            fmt::count(
                rows.iter().map(|r| r.commits_today).sum::<u32>(),
                "commit",
                "commits"
            )
        );
        IndexPage {
            rows,
            head: crate::dochead::DocHead {
                title: "Repositories".to_owned(),
                meta: sub,
                ..crate::dochead::DocHead::default()
            },
        }
        .render()
    })
    .await??;
    Ok(Html(html).into_response())
}
