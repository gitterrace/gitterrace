//! The activity page, `/stats`: totals against the period before, rhythm and the punch card.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::Period;

use crate::{
    charts,
    error::Result,
    filter::FilterBar,
    fmt::{self},
    insights::{InsightsQuery, OptLink, cached, insights, period_of, scoped},
    routes::Shared,
    views::{self, Chrome, Tabs},
};

fn delta(now: u64, before: Option<u64>) -> (String, &'static str) {
    let Some(b) = before else {
        return (String::new(), "");
    };
    if b == 0 {
        return (
            if now == 0 {
                String::new()
            } else {
                "new".to_owned()
            },
            "up",
        );
    }
    let pct = (now as f64 - b as f64) / b as f64 * 100.0;
    (format!("{pct:+.0}%"), if pct >= 0.0 { "up" } else { "dn" })
}

struct PulseRow {
    label: &'static str,
    now: String,
    before: String,
    delta: String,
    tone: &'static str,
}

#[derive(Template)]
#[template(path = "stats.html")]
struct ActivityPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    rows: Vec<PulseRow>,
    span: String,
    compared: bool,
    unit: &'static str,
    series: String,
    heatmap: String,
}

/// The landing insights page: the period against the one before it, commits per bucket and
/// when commits happen.
pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "a", raw, move |a, r, c| {
        let f = &q.filter;
        let recs = scoped(f, r);
        let d = a.defaults();
        let ins = insights(a, &recs, q.options(&d));
        let period = period_of(&q, &d, Period::All);
        let s = ins.period_summary(period);
        let prev = s.previous.as_ref();
        let row = |label, now: u64, before: Option<u64>| {
            let (d, tone) = delta(now, before);
            PulseRow {
                label,
                now: fmt::thousands(now),
                before: before.map_or_else(|| "-".to_owned(), fmt::thousands),
                delta: d,
                tone,
            }
        };
        let rows = vec![
            row(
                "Commits",
                s.current.commits as u64,
                prev.map(|p| p.commits as u64),
            ),
            row(
                "Authors",
                s.current.authors as u64,
                prev.map(|p| p.authors as u64),
            ),
            row(
                "Files changed",
                s.current.files as u64,
                prev.map(|p| p.files as u64),
            ),
            row(
                "Lines added",
                s.current.additions,
                prev.map(|p| p.additions),
            ),
            row(
                "Lines removed",
                s.current.deletions,
                prev.map(|p| p.deletions),
            ),
        ];
        let span = match s.from {
            Some(from) => format!(
                "{} to {}, against the {} before",
                fmt::day(from),
                fmt::day(s.to - 1),
                fmt::count(period.days().unwrap_or(0), "day", "days")
            ),
            None => format!("All history up to {}", fmt::day(s.to - 1)),
        };
        let rhythm = ins.rhythm(period);
        let series: Vec<_> = rhythm
            .series
            .iter()
            .map(|p| (p.label.clone(), u64::from(p.commits)))
            .collect();
        ActivityPage {
            c,
            tabs: views::stats_tabs("a"),
            fb: f.bar("/stats", r),
            opts: q.row(&d, "/stats"),
            rows,
            span,
            compared: prev.is_some(),
            unit: rhythm.unit.noun(),
            series: charts::bars(&series),
            heatmap: charts::heatmap(&rhythm.punch_card, rhythm.punch_max),
        }
    })
    .await
}
