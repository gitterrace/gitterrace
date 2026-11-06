//! The activity page, `/stats`: totals against the period before, rhythm and the punch card,
//! then the commit types and code frequency sections.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::feed::SystemClock;
use gitterrace_index::{CountContext, Period, PeriodStats, Scope, ScopeParams};
use serde::Deserialize;

use crate::{
    charts,
    error::Result,
    filter::{Chip, FilterBar},
    fmt::{self},
    insights::{InsightsQuery, OptLink, cached_keyed, insights, period_of},
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

/// Label of the commit total: insights count non-merge commits, the overview counts all.
const COMMITS: &str = "Commits without merges";
/// The same total with the merges option on.
const COMMITS_WITH_MERGES: &str = "Commits";

/// The date range a bucket link narrows the page to: `?from=YYYY-MM-DD&to=YYYY-MM-DD`, both
/// ends inclusive.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RangeQuery {
    from: Option<String>,
    to: Option<String>,
}

/// `YYYY-MM-DD` as days since the epoch.
fn parse_day(s: &str) -> Option<i64> {
    let mut it = s.splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d) && s.len() == 10)
        .then(|| gitterrace_git::days_from_civil(y, m, d))
}

impl RangeQuery {
    /// The range as `(from, to)` dates and days, or `None` when absent or malformed.
    fn parsed(&self) -> Option<(&str, &str, i64, i64)> {
        let (f, t) = (self.from.as_deref()?, self.to.as_deref()?);
        let (a, b) = (parse_day(f)?, parse_day(t)?);
        (a <= b).then_some((f, t, a, b))
    }
}

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

struct PulseRow {
    label: &'static str,
    tip: &'static str,
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
    lead: String,
    rows: Vec<PulseRow>,
    span: String,
    empty: bool,
    compared: bool,
    unit: &'static str,
    series: crate::charts::view::Bars,
    heatmap: crate::charts::view::Heatmap,
    range: Option<RangeCard>,
    /// The commit types section, markup from `commit_types`.
    types: String,
    /// The code frequency section, markup from `frequency`.
    frequency: String,
}

/// `?page=` of the commit list in the commit types section.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ListPage {
    page: Option<String>,
}

struct RangeCard {
    title: String,
    total: String,
    weekdays: crate::charts::view::Bars,
}

fn span_text(from: Option<i64>, to: i64, days: Option<i64>) -> String {
    from.map_or_else(
        || format!("All history up to {}", fmt::day(to - 1)),
        |from| {
            format!(
                "{} to {}, against the {} before",
                fmt::day(from),
                fmt::day(to - 1),
                fmt::count(days.unwrap_or(0), "day", "days")
            )
        },
    )
}

/// The weekday split of the narrowed range.
fn range_card(from: &str, to: &str, by_weekday: &[u32; 7]) -> RangeCard {
    let days: Vec<(String, u64)> = WEEKDAYS
        .iter()
        .zip(by_weekday)
        .map(|(n, v)| ((*n).to_owned(), u64::from(*v)))
        .collect();
    RangeCard {
        title: format!("{from} to {to}"),
        total: fmt::count(
            by_weekday.iter().map(|v| u64::from(*v)).sum::<u64>(),
            "commit",
            "commits",
        ),
        weekdays: charts::bars(&days, crate::charttip::COMMITS),
    }
}

/// One counted set over the whole history, anchored on its newest commit: a chip narrows the
/// commits, never the days "last week" means. A bar's range replaces the period.
fn scope_of(
    q: &InsightsQuery,
    d: &crate::insights::Defaults,
    period: Period,
    sel: Option<(&str, &str, i64, i64)>,
) -> Scope {
    let params = ScopeParams {
        period: Some(sel.map_or(period, |_| Period::All).as_str().to_owned()),
        from: sel.map(|(f, _, _, _)| f.to_owned()),
        to: sel.map(|(_, t, _, _)| t.to_owned()),
        select: q.filter.selection(),
        options: q.opts.clone(),
    };
    Scope::from_params(&params, d.clone().into_options(), period)
        .unwrap_or_else(|_| unreachable!("the period and the range are parsed already"))
}

/// The pulse table: each count of the period beside the one before it.
fn pulse_rows(now: &PeriodStats, prev: Option<&PeriodStats>, merges: bool) -> Vec<PulseRow> {
    let commits = if merges { COMMITS_WITH_MERGES } else { COMMITS };
    let row = |label: &'static str, now: u64, before: Option<u64>| {
        let (d, tone) = delta(now, before);
        PulseRow {
            label,
            tip: if label == COMMITS && !merges {
                "Merges are left out: a merge repeats the changes of its branch."
            } else {
                ""
            },
            now: fmt::thousands(now),
            before: before.map_or_else(|| "-".to_owned(), fmt::thousands),
            delta: d,
            tone,
        }
    };
    vec![
        row(commits, now.commits as u64, prev.map(|p| p.commits as u64)),
        row(
            "Authors",
            now.authors as u64,
            prev.map(|p| p.authors as u64),
        ),
        row(
            "Files changed",
            now.files as u64,
            prev.map(|p| p.files as u64),
        ),
        row("Lines added", now.additions, prev.map(|p| p.additions)),
        row("Lines removed", now.deletions, prev.map(|p| p.deletions)),
    ]
}

/// The landing insights page: the period against the one before it, commits per bucket and
/// when commits happen.
pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
    Query(rq): Query<RangeQuery>,
    Query(lp): Query<ListPage>,
) -> Result<Html<String>> {
    // The memo key keeps only the shared parameters: the range is part of this page's key.
    let mut key = crate::app::App::canonical_query(raw.as_deref());
    if let Some((f, t, _, _)) = rq.parsed() {
        key = format!("{key}&from={f}&to={t}");
    }
    cached_keyed(&app, "a", key, move |a, r, c| {
        let f = &q.filter;
        let d = a.defaults();
        let period = period_of(&q, &d, Period::All);
        let sel = rq.parsed();
        let scope = scope_of(&q, &d, period, sel);
        let ins = insights(a, r, scope.options().clone());
        let counted = ins.counted(&scope, CountContext::new(&**r, &SystemClock));
        let (s, rhythm) = counted.activity();
        let prev = s.previous.as_ref();
        let rows = pulse_rows(&s.current, prev, scope.options().include_merges);
        let lead = crate::lead::activity(
            s.current.commits as u64,
            s.current.authors as u64,
            prev.map(|p| p.commits as u64),
        );
        let span_days = sel.map_or_else(|| period.days(), |(_, _, a, b)| Some(b - a + 1));
        let span = span_text(s.from, s.to, span_days);
        let series: Vec<_> = rhythm
            .series
            .iter()
            .map(|p| (p.label.clone(), u64::from(p.commits)))
            .collect();
        let links: Vec<String> = rhythm
            .series
            .iter()
            .zip(&rhythm.ends)
            .map(|(p, end)| q.range_link(&d, "/stats", &p.key, end))
            .collect();
        let mut fb = f.bar("/stats", &**r, period);
        let range = sel.map(|(from, to, _, _)| {
            fb.chips.push(Chip {
                dim: "range",
                label: format!("{from} to {to}"),
                href: q.unranged_link(&d, "/stats"),
            });
            range_card(from, to, &rhythm.by_weekday)
        });
        let types_html = if crate::features::pages().contains(&"types") {
            crate::commit_types::section(a, r, &q, lp.page.as_deref())
        } else {
            String::new()
        };
        let frequency_html = if crate::features::pages().contains(&"frequency") {
            crate::frequency::section(a, r, &q)
        } else {
            String::new()
        };
        fb.jump = crate::markdown::outline_menu(
            &crate::sections::jumps(&[
                ("activity-counts", "Counts", true),
                ("activity-series", "Over time", true),
                ("activity-hours", "When commits happen", true),
                ("types", "Commit types", !types_html.is_empty()),
                ("frequency", "Code frequency", !frequency_html.is_empty()),
            ]),
            false,
        );
        ActivityPage {
            types: types_html,
            frequency: frequency_html,
            c,
            tabs: views::stats_tabs("a"),
            fb,
            opts: q.row(&d, "/stats"),
            lead,
            rows,
            span,
            empty: s.current.commits == 0,
            compared: prev.is_some(),
            unit: rhythm.unit.noun(),
            series: charts::linked_bars(&series, &links),
            range,
            heatmap: charts::heatmap(&rhythm.punch_card, rhythm.punch_max),
            // `insights.pages` still names each section: one left out is not shown.
        }
    })
    .await
}
