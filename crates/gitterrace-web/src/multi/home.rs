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

/// Threads reading rows at once: enough to overlap the trees' disk reads, few enough that a
/// home page of a hundred repositories does not crowd the server's other requests.
const READERS: usize = 8;

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
    /// Uncommitted work in the working tree: `None` where the tree is not shown, else whether
    /// the chip shows at once; a row without a fresh answer starts hidden and `home.js` fills it.
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

/// `f` over `items` on up to [`READERS`] threads, the answers in the order of `items`.
/// Each thread takes every `READERS`th item, so a slow repository holds up only its stripe.
fn in_order<I: Sync, T: Send>(items: &[I], f: impl Fn(&I) -> T + Sync) -> Vec<T> {
    let mut out: Vec<Option<T>> = std::iter::repeat_with(|| None).take(items.len()).collect();
    std::thread::scope(|sc| {
        let f = &f;
        let stripes: Vec<_> = (0..READERS.min(items.len()))
            .map(|k| {
                sc.spawn(move || {
                    let mine = items.iter().enumerate().skip(k).step_by(READERS);
                    mine.map(|(i, it)| (i, f(it))).collect::<Vec<_>>()
                })
            })
            .collect();
        for h in stripes {
            // A panic in one read is the request's panic, as when the rows were read inline.
            let got = h.join().unwrap_or_else(|p| std::panic::resume_unwind(p));
            for (i, v) in got {
                out[i] = Some(v);
            }
        }
    });
    out.into_iter().flatten().collect()
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
        let show_tree = shows_tree(&table);
        let served = table.snapshot();
        let reads = in_order(&served, |s| {
            (s.recent(now), show_tree.then(|| s.known_dirty()))
        });
        let rows: Vec<IndexRow> = served
            .iter()
            .zip(reads)
            .map(|(s, (r, dirty))| {
                let href = format!("/r/{}/", s.name);
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
                    dirty: dirty.map(|d| d.unwrap_or(false)),
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
                page_head: true,
                ..crate::dochead::DocHead::default()
            },
        }
        .render()
    })
    .await??;
    Ok(Html(html).into_response())
}

/// Whether the home page shows working tree chips: never on a public server.
fn shows_tree(table: &Table) -> bool {
    !table.env.public() && crate::features::on(gitterrace_config::Feature::Worktree)
}

/// `/_dirty`, fetched by `home.js` after the rows render, so a cold status scan of every
/// tree never holds the page: `{"name": true}` for every repository with uncommitted work,
/// read on the home page's threads. Each answer is reused for `DIRTY_TTL`. Not public: it
/// describes trees.
pub(super) async fn dirty(State(table): State<Arc<Table>>) -> Response {
    let names = tokio::task::spawn_blocking(move || {
        if !shows_tree(&table) {
            return Vec::new();
        }
        let served = table.snapshot();
        let flags = in_order(&served, |s| s.dirty());
        served
            .iter()
            .zip(flags)
            .filter(|(_, d)| *d)
            .map(|(s, _)| s.name.clone())
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let map: serde_json::Map<String, serde_json::Value> = names
        .into_iter()
        .map(|n| (n, serde_json::Value::Bool(true)))
        .collect();
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        axum::Json(map),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    /// Reads finishing out of order (the early items sleep longest) still come back in the
    /// order of the items, also past one stripe per thread.
    #[test]
    fn parallel_reads_keep_the_row_order() {
        let items: Vec<u64> = (0..37).collect();
        let got = super::in_order(&items, |n| {
            std::thread::sleep(std::time::Duration::from_millis(40 - n));
            n * 10
        });
        assert_eq!(got, items.iter().map(|n| n * 10).collect::<Vec<_>>());
    }
}
