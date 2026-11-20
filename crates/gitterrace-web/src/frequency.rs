//! The code frequency section of the activity page: lines added and removed per bucket, and
//! rolling rework.

use askama::Template;
use gitterrace_index::{ActivityFold, rework_share, rework_verdict};

use crate::{charts, fmt, insights::Kpi};

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

pub(crate) fn section(fold: &ActivityFold) -> String {
    {
        let (unit, weeks, sum) = &fold.frequency;
        let (unit, sum) = (*unit, *sum);
        let (add, del, rew) = (sum.additions, sum.deletions, sum.rewritten);
        let points: Vec<_> =
            weeks.iter().map(|w| (w.label.clone(), w.additions, w.deletions)).collect();
        let rw = &fold.rework;
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
                &rw.points.iter().map(|p| (p.label.clone(), p.share)).collect::<Vec<_>>(),
            ),
            verdict,
        }
        .render()
        .unwrap_or_default()
    }
}
