//! The hotspots page, `/stats/hotspots`: files ranked by changes and size, then the patterns
//! sections (coupled files, code age, commit sizes, releases).

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::Period;

use crate::{
    charts,
    error::Result,
    filter::FilterBar,
    fmt::{self},
    insights::{
        InsightsQuery, Kpi, OptLink, cached, counted, head_lines, insights, page_scope, period_of,
    },
    routes::Shared,
    views::{self, Chrome, Tabs},
};

const TOP_HOT: usize = 50;
const SCATTER: usize = 400;

struct HotRow {
    path: String,
    /// The path split at `/`, for the `_path.html` macro.
    shown: Vec<String>,
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
    empty: bool,
    lead: String,
    kpis: Vec<Kpi>,
    scatter: crate::charts::view::Scatter,
    rows: Vec<HotRow>,
    /// The patterns sections, markup from `patterns`.
    patterns: String,
}

pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "ho", raw, move |a, r, c| {
        let f = &q.filter;
        let d = a.defaults();
        let period = period_of(&q, &d, Period::All);
        let scope = page_scope(&q, &d, period);
        let ins = insights(a, r, scope.options().clone());
        let set = counted(&ins, &scope, r);
        let files = head_lines(a);
        let rep = ins.hotspots_in(&files, &set);
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
                name: h.path.clone(),
                title: format!(
                    "{}, {}, {}",
                    crate::charttip::part(
                        &h.path,
                        h.changes as u64,
                        rep.total_changes as u64,
                        crate::charttip::CHANGES
                    ),
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
                shown: fmt::path_parts(&h.path),
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
        let patterns = if crate::features::pages().contains(&"patterns") {
            crate::patterns::section(a, r, &q)
        } else {
            String::new()
        };
        let empty = set.stream().next().is_none();
        let jump = crate::markdown::outline_menu(
            &crate::sections::jumps(&[
                ("hotspot-map", "Frequency against size", !empty),
                ("hotspot-files", "Hotspots", !empty),
                ("coupled", "Coupled files", !patterns.is_empty()),
                ("code-age", "Code age", !patterns.is_empty()),
                ("sizes", "Commit sizes", !patterns.is_empty()),
                ("releases", "Releases", !patterns.is_empty()),
            ]),
            false,
        );
        let mut fb = f.bar("/stats/hotspots", &**r, period);
        fb.jump = jump;
        HotspotsPage {
            c,
            tabs: views::stats_tabs("ho"),
            fb,
            opts: q.row(&d, "/stats/hotspots"),
            empty,
            lead: crate::lead::hotspots(rep.files.len(), rep.concentration),
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
            patterns,
        }
    })
    .await
}
