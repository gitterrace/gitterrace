//! The code frequency section of the activity page: lines added and removed per bucket, and
//! rolling rework.

use askama::Template;
use gitterrace_index::{Period, rework_share, rework_verdict};

use crate::{
    charts, fmt,
    insights::{InsightsQuery, Kpi, Src, counted, insights, page_scope, period_of},
};

#[derive(Template)]
#[template(path = "frequency_section.html")]
struct FrequencySection {
    kpis: Vec<Kpi>,
    unit: &'static str,
    window: String,
    weekly: crate::charts::view::Diverging,
    rework: crate::charts::view::PercentLine,
    verdict: String,
}

/// The section's markup for the commits `q` selects.
pub(crate) fn section(a: &crate::app::App, r: &Src, q: &InsightsQuery) -> String {
    {
        let d = a.defaults();
        let period = period_of(q, &d, Period::All);
        let scope = page_scope(q, &d, period);
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
        FrequencySection {
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
        .render()
        .unwrap_or_default()
    }
}
