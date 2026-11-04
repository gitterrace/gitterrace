//! The code frequency page: lines added and removed per bucket, and rolling rework.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::{Period, rework_share, rework_verdict};

use crate::{
    charts,
    error::Result,
    filter::FilterBar,
    fmt,
    insights::{InsightsQuery, Kpi, OptLink, cached, counted, insights, page_scope, period_of},
    routes::Shared,
    views::{self, Chrome, Tabs},
};

#[derive(Template)]
#[template(path = "frequency.html")]
struct FrequencyPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    kpis: Vec<Kpi>,
    unit: &'static str,
    window: String,
    weekly: String,
    rework: String,
    verdict: String,
}

pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "cf", raw, move |a, r, c| {
        let d = a.defaults();
        let f = &q.filter;
        let fb = f.bar("/stats/frequency", &**r);
        let period = period_of(&q, &d, Period::All);
        let scope = page_scope(&q, &d, period);
        let ins = insights(a, r, scope.options().clone());
        let set = counted(&ins, &scope, r);
        let (unit, weeks) = set.frequency();
        let (add, del, rew) = weeks.iter().fold((0, 0, 0), |t, w| {
            (t.0 + w.additions, t.1 + w.deletions, t.2 + w.rewritten)
        });
        let points: Vec<_> = weeks
            .iter()
            .map(|w| (w.label.clone(), w.additions, w.deletions))
            .collect();
        let rw = set.rework();
        let verdict = rw.points.iter().rev().find_map(|p| p.share).map_or_else(
            || "No changes in this selection.".to_owned(),
            |s| {
                format!(
                    "Latest window of {}: {:.0}% of changed lines replaced existing code, {}.",
                    fmt::count(rw.window, "commit", "commits"),
                    s * 100.0,
                    rework_verdict(s)
                )
            },
        );
        FrequencyPage {
            c,
            tabs: views::stats_tabs("cf"),
            fb,
            opts: q.row(&d, "/stats/frequency"),
            kpis: vec![
                Kpi("Lines added", fmt::thousands(add)),
                Kpi("Lines removed", fmt::thousands(del)),
                Kpi(
                    "Net",
                    format!(
                        "{}{}",
                        if add >= del { "+" } else { "-" },
                        fmt::thousands(add.abs_diff(del))
                    ),
                ),
                Kpi(
                    "Rework overall",
                    rework_share(add, del, rew)
                        .map_or_else(|| "-".to_owned(), |s| format!("{:.0}%", s * 100.0)),
                ),
            ],
            unit: unit.noun(),
            window: fmt::count(rw.window, "commit", "commits"),
            weekly: charts::diverging(
                &points,
                &format!("Additions and deletions per {}", unit.noun()),
            ),
            rework: charts::percent_line(
                &rw.points
                    .iter()
                    .map(|p| (p.label.clone(), p.share))
                    .collect::<Vec<_>>(),
            ),
            verdict,
        }
    })
    .await
}
