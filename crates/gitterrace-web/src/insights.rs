//! Insights pages over the shared filter: code frequency and contributors.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use serde::Deserialize;
use gitterrace_index::{CommitRecord, Insights, Options, Period, rework_share, rework_verdict};

use crate::{
    app::{App, blocking},
    charts,
    error::Result,
    filter::{Dim, Filter, FilterBar},
    fmt,
    routes::{Shared, with_records},
    views::{self, Chrome, Nav},
};

/// Commits in the rolling window of the rework ratio.
const REWORK_WINDOW: usize = 100;

#[derive(Deserialize, Default)]
pub(crate) struct InsightsQuery {
    #[serde(flatten)]
    filter: Filter,
    /// `bots=1` includes bot accounts.
    bots: Option<String>,
}

fn insights<'a>(a: &App, recs: &'a [CommitRecord]) -> Insights<'a> {
    let head = a.repo.resolve("HEAD").unwrap_or_default();
    let ignore = gitterrace_git::IgnoreSet::at(&a.repo, &head).ok();
    Insights::new(recs, Options::default(), ignore)
}

/// Memoizes the rendered page per tip and query, building on a miss.
async fn cached<T: askama::Template + Send + 'static>(
    app: &Arc<App>,
    tab: &'static str,
    raw: Option<String>,
    build: impl FnOnce(&App, &[CommitRecord], Chrome) -> T + Send + 'static,
) -> Result<Html<String>> {
    let key = blocking(app, move |a| Ok(a.memo_key(tab, raw.as_deref()))).await?;
    if let Some(h) = app.memo_get(&key) {
        return Ok(Html((*h).clone()));
    }
    let html = with_records(app, tab, build).await?;
    if app.ready() {
        app.memo_put(key, Arc::new(html.0.clone()));
    }
    Ok(html)
}

struct Kpi(&'static str, String);

#[derive(Template)]
#[template(path = "code_frequency.html")]
struct CodeFrequencyPage {
    c: Chrome,
    tabs: Vec<Nav>,
    fb: FilterBar,
    kpis: Vec<Kpi>,
    weekly: String,
    rework: String,
    verdict: String,
}

pub(crate) async fn code_frequency(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "cf", raw, move |a, r, c| {
        let f = &q.filter;
        let fb = f.bar("/stats/frequency", r);
        let recs = f.apply(r);
        let ins = insights(a, &recs);
        let weeks = ins.code_frequency();
        let (add, del, rew) = weeks.iter().fold((0, 0, 0), |t, w| {
            (t.0 + w.additions, t.1 + w.deletions, t.2 + w.rewritten)
        });
        let points: Vec<_> = weeks
            .iter()
            .map(|w| (w.key.clone(), w.additions, w.deletions))
            .collect();
        let rw = ins.rework(REWORK_WINDOW);
        let verdict = rw.iter().rev().find_map(|p| p.share).map_or_else(
            || "No changes in this selection.".to_owned(),
            |s| {
                format!(
                    "Latest window: {:.0}% of changed lines replaced existing code, {}.",
                    s * 100.0,
                    rework_verdict(s)
                )
            },
        );
        CodeFrequencyPage {
            c,
            tabs: views::stats_tabs("cf"),
            fb,
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
            weekly: charts::diverging(&points),
            rework: charts::percent_line(
                &rw.iter()
                    .map(|p| (p.key.clone(), p.share))
                    .collect::<Vec<_>>(),
            ),
            verdict,
        }
    })
    .await
}

struct PersonRow {
    name: String,
    href: String,
    log_href: String,
    first_time: bool,
    bot: bool,
    commits: String,
    additions: String,
    deletions: String,
    first: String,
    last: String,
    spark: String,
    hue: u32,
    initial: String,
}

#[derive(Template)]
#[template(path = "contributors.html")]
struct ContributorsPage {
    c: Chrome,
    tabs: Vec<Nav>,
    fb: FilterBar,
    kpis: Vec<Kpi>,
    people: Vec<PersonRow>,
    bots_href: String,
    bots: bool,
}

pub(crate) async fn contributors(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "co", raw, move |a, r, c| {
        let f = &q.filter;
        let bots = q.bots.as_deref() == Some("1");
        let fb = f.bar("/stats/contributors", r);
        // The period is applied by Insights so first-time contributors are judged against all
        // history that the other filters keep.
        let recs = f.with(Dim::Period, None).apply(r);
        let ins = insights(a, &recs);
        let period = f.period_value().unwrap_or(Period::All);
        let report = ins.contributors(period);
        let people: Vec<PersonRow> = report
            .contributors
            .iter()
            .filter(|p| bots || !p.bot)
            .map(|p| {
                let email = p.emails.first().map_or(p.name.as_str(), String::as_str);
                PersonRow {
                    name: p.name.clone(),
                    href: f.link("/stats/contributors", Dim::Author, email),
                    log_href: views::author_href(email),
                    first_time: p.first_time,
                    bot: p.bot,
                    commits: fmt::thousands(p.commits),
                    additions: fmt::thousands(p.additions),
                    deletions: fmt::thousands(p.deletions),
                    first: fmt::day(p.first_commit),
                    last: fmt::day(p.last_commit),
                    spark: charts::sparkline(&p.weekly, "s1"),
                    hue: views::hue(&p.name),
                    initial: views::initial(&p.name),
                }
            })
            .collect();
        let (add, del): (u64, u64) = report
            .contributors
            .iter()
            .filter(|p| bots || !p.bot)
            .fold((0, 0), |t, p| (t.0 + p.additions, t.1 + p.deletions));
        let query = f.query(&if bots { vec![] } else { vec![("bots", "1")] });
        ContributorsPage {
            c,
            tabs: views::stats_tabs("co"),
            fb,
            kpis: vec![
                Kpi("Contributors", fmt::thousands(people.len())),
                Kpi(
                    "New in period",
                    fmt::thousands(people.iter().filter(|p| p.first_time).count()),
                ),
                Kpi("Lines added", fmt::thousands(add)),
                Kpi("Lines removed", fmt::thousands(del)),
            ],
            people,
            bots_href: fmt::href(&format!("/stats/contributors{query}")),
            bots,
        }
    })
    .await
}
