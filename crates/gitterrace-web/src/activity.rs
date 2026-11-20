//! The activity page, `/stats`: totals against the period before, rhythm and the punch card,
//! then the commit types and code frequency sections.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::{BucketUnit, Period, PeriodStats, Rhythm, Scope, ScopeParams};
use serde::Deserialize;

use crate::{
    charts,
    error::Result,
    filter::FilterBar,
    fmt::{self},
    insights::{InsightsQuery, Kpi, cached_keyed, insights, period_of},
    pagehead::PageHead,
    routes::Shared,
    views::Chrome,
};

fn delta(now: u64, before: Option<u64>) -> (String, &'static str) {
    let Some(b) = before else {
        return (String::new(), "");
    };
    if b == 0 {
        return (if now == 0 { String::new() } else { "new".to_owned() }, "up");
    }
    let pct = (now as f64 - b as f64) / b as f64 * 100.0;
    (format!("{pct:+.0}%"), if pct >= 0.0 { "up" } else { "dn" })
}

/// Label of the commit total: insights count non-merge commits, the overview counts all.
const COMMITS: &str = "Commits without merges";
/// The same total with the merges option on.
const COMMITS_WITH_MERGES: &str = "Commits";

const WEEKDAYS: [&str; 7] =
    ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

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
    kpis: Vec<Kpi>,
    head: PageHead,
    fb: FilterBar,
    rows: Vec<PulseRow>,
    span: String,
    empty: bool,
    compared: bool,
    unit: &'static str,
    series: crate::charts::view::Bars,
    heatmap: crate::charts::view::Heatmap,
    range: Option<RangeCard>,
    types: String,
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

/// The big numbers of the period: commits, authors, how many of its days or weeks saw a commit,
/// and the busiest one. From the series the page charts, so no further read.
fn activity_kpis(now: &PeriodStats, rhythm: &Rhythm) -> Vec<Kpi> {
    let active = rhythm.series.iter().filter(|p| p.commits > 0).count();
    let busiest = rhythm
        .series
        .iter()
        .filter(|p| p.commits > 0)
        .max_by_key(|p| p.commits)
        .map_or_else(|| "-".to_owned(), |p| p.label.clone());
    vec![
        Kpi("Commits", fmt::thousands(now.commits)),
        Kpi("Authors", fmt::thousands(now.authors)),
        Kpi(
            active_label(rhythm.unit),
            format!("{} of {}", fmt::thousands(active), rhythm.series.len()),
        ),
        Kpi(busiest_label(rhythm.unit), busiest),
    ]
}

/// "Active days", "Active weeks" and so on, for the KPI caption.
const fn active_label(unit: BucketUnit) -> &'static str {
    match unit {
        BucketUnit::Day => "Active days",
        BucketUnit::Week => "Active weeks",
        BucketUnit::Month => "Active months",
        BucketUnit::Quarter => "Active quarters",
        BucketUnit::Year => "Active years",
    }
}

/// "Busiest day", "Busiest week" and so on.
const fn busiest_label(unit: BucketUnit) -> &'static str {
    match unit {
        BucketUnit::Day => "Busiest day",
        BucketUnit::Week => "Busiest week",
        BucketUnit::Month => "Busiest month",
        BucketUnit::Quarter => "Busiest quarter",
        BucketUnit::Year => "Busiest year",
    }
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
    let days: Vec<(String, u64)> =
        WEEKDAYS.iter().zip(by_weekday).map(|(n, v)| ((*n).to_owned(), u64::from(*v))).collect();
    RangeCard {
        title: format!("{from} to {to}"),
        total: fmt::count(
            by_weekday.iter().map(|v| u64::from(*v)).sum::<u64>(),
            "commit",
            "commits",
        ),
        weekdays: charts::bars("Commits by weekday", &days, crate::charttip::COMMITS),
    }
}

/// One counted set over the whole history, anchored on its newest commit: a chip narrows the
/// commits, never the days "last week" means. A bar's range replaces the period.
fn scope_of(
    q: &InsightsQuery,
    d: &crate::insights::Defaults,
    period: Period,
    sel: Option<(i64, i64)>,
) -> Scope {
    let params = ScopeParams {
        period: Some(sel.map_or(period, |_| Period::All).as_str().to_owned()),
        from: sel.map(|(f, _)| gitterrace_git::ymd(f)),
        to: sel.map(|(_, t)| gitterrace_git::ymd(t)),
        select: q.filter.unranged().selection(),
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
        row("Authors", now.authors as u64, prev.map(|p| p.authors as u64)),
        row("Files changed", now.files as u64, prev.map(|p| p.files as u64)),
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
    Query(lp): Query<ListPage>,
) -> Result<Html<String>> {
    // The memo key keeps the shared parameters, and the range keys `from` and `to` are among
    // them: an older link's range moves the key as `q` does.
    let key = crate::app::App::canonical_query(raw.as_deref());
    cached_keyed(&app, "a", key, move |a, r, c| {
        let f = &q.filter;
        let d = a.defaults();
        let period = period_of(&q, &d, Period::All);
        let sel = f.range();
        let scope = scope_of(&q, &d, period, sel);
        let ins = insights(a, r, scope.options().clone());
        let counted = crate::insights::counted(&ins, &scope, r);
        // One walk feeds the page and both sections. The sections count the page's period
        // without the range, so a selected range gives them a fold of their own.
        let fold = counted.activity_fold();
        let plain = scope_of(&q, &d, period, None);
        let plain_set = crate::insights::counted(&ins, &plain, r);
        let plain_fold = sel.map(|_| plain_set.activity_fold());
        let (sec_set, sec_fold) =
            plain_fold.as_ref().map_or((&counted, &fold), |f| (&plain_set, f));
        let (s, rhythm) = (&fold.summary, &fold.rhythm);
        let prev = s.previous.as_ref();
        let rows = pulse_rows(&s.current, prev, scope.options().include_merges);
        let lead = crate::lead::activity(
            s.current.commits as u64,
            s.current.authors as u64,
            prev.map(|p| p.commits as u64),
        );
        let span_days = sel.map_or_else(|| period.days(), |(a, b)| Some(b - a + 1));
        let span = span_text(s.from, s.to, span_days);
        let series: Vec<_> =
            rhythm.series.iter().map(|p| (p.label.clone(), u64::from(p.commits))).collect();
        let links: Vec<String> = rhythm
            .series
            .iter()
            .zip(&rhythm.ends)
            .map(|(p, end)| q.range_link(&d, "/stats", &p.key, end))
            .collect();
        let mut fb = f.bar("/stats", &**r, period);
        let range = sel.map(|(from, to)| {
            let (from, to) = (gitterrace_git::ymd(from), gitterrace_git::ymd(to));
            range_card(&from, &to, &rhythm.by_weekday)
        });
        let types_html = if crate::features::pages().contains(&"types") {
            crate::commit_types::section(a, &q, lp.page.as_deref(), sec_set, sec_fold)
        } else {
            String::new()
        };
        let frequency_html = if crate::features::pages().contains(&"frequency") {
            crate::frequency::section(sec_fold)
        } else {
            String::new()
        };
        fb.jump = crate::markdown::outline_menu(
            &crate::markdown::section_jumps(&[
                ("activity-counts", "Counts", true),
                ("activity-series", "Over time", true),
                ("activity-hours", "When commits happen", true),
                ("types", "Commit types", !types_html.is_empty()),
                ("frequency", "Code frequency", !frequency_html.is_empty()),
            ]),
            false,
        );
        let kpis = activity_kpis(&s.current, rhythm);
        ActivityPage {
            kpis,
            types: types_html,
            frequency: frequency_html,
            c,
            head: crate::pagehead::insights_head(
                "a",
                crate::pagehead::lead_tagline(&lead),
                fb.toolbar(&q.row(&d, "/stats")),
            ),
            fb,
            rows,
            span,
            empty: s.current.commits == 0,
            compared: prev.is_some(),
            unit: rhythm.unit.noun(),
            series: charts::linked_bars(
                &format!("Commits per {}", rhythm.unit.noun()),
                &series,
                &links,
            ),
            range,
            heatmap: charts::heatmap(&rhythm.punch_card, rhythm.punch_max),
            // `insights.pages` still names each section: one left out is not shown.
        }
    })
    .await
}
