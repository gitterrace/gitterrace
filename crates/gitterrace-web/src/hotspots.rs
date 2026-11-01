//! The hotspots page, `/stats/hotspots`: files ranked by changes and size.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::Period;

use crate::{
    charts,
    error::Result,
    filter::FilterBar,
    fmt::{self},
    insights::{InsightsQuery, Kpi, OptLink, cached, head_lines, insights, period_of, scoped},
    routes::Shared,
    views::{self, Chrome, Tabs},
};

const TOP_HOT: usize = 50;
const SCATTER: usize = 400;

struct HotRow {
    path: String,
    href: String,
    language: String,
    lines: String,
    changes: String,
    authors: String,
    last: String,
    score: String,
    pct: u32,
}

#[derive(Template)]
#[template(path = "hotspots.html")]
struct HotspotsPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    kpis: Vec<Kpi>,
    scatter: String,
    rows: Vec<HotRow>,
}

pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "ho", raw, move |a, r, c| {
        let f = &q.filter;
        let recs = scoped(f, r);
        let d = a.defaults();
        let ins = insights(a, &recs, q.options(&d));
        let files = head_lines(a);
        let rep = ins.hotspots(&files, period_of(&q, &d, Period::All));
        let max = rep.files.first().map_or(0.0, |h| h.score);
        let dots: Vec<_> = rep
            .files
            .iter()
            .take(SCATTER)
            .map(|h| charts::Dot {
                x: h.changes as u64,
                y: u64::from(h.lines),
                size: h.authors as u64,
                href: views::file_link(&h.path),
                title: format!(
                    "{}: {}, {}, {}",
                    h.path,
                    fmt::count(h.changes, "change", "changes"),
                    fmt::count(h.lines, "line", "lines"),
                    fmt::count(h.authors, "author", "authors")
                ),
            })
            .collect();
        let rows = rep
            .files
            .iter()
            .take(TOP_HOT)
            .map(|h| HotRow {
                path: h.path.clone(),
                href: views::file_link(&h.path),
                language: h.language.clone(),
                lines: fmt::thousands(h.lines),
                changes: fmt::thousands(h.changes),
                authors: fmt::thousands(h.authors),
                last: fmt::day(h.last_change),
                score: fmt::thousands(h.score.round() as u64),
                pct: if max > 0.0 {
                    (h.score / max * 100.0).max(1.0) as u32
                } else {
                    0
                },
            })
            .collect();
        HotspotsPage {
            c,
            tabs: views::stats_tabs("ho"),
            fb: f.bar("/stats/hotspots", r),
            opts: q.row(&d, "/stats/hotspots"),
            kpis: vec![
                Kpi("Files changed", fmt::thousands(rep.files.len())),
                Kpi("Changes", fmt::thousands(rep.total_changes)),
                Kpi(
                    "Top tenth takes",
                    format!("{:.0}%", rep.concentration * 100.0),
                ),
                Kpi(
                    "Single-author files",
                    fmt::thousands(rep.single_author_files),
                ),
            ],
            scatter: charts::scatter(&dots, "changes"),
            rows,
        }
    })
    .await
}
