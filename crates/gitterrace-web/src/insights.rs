//! Insights pages over the shared filter: code frequency and contributors.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::{CommitRecord, Insights, Options, Period, rework_share, rework_verdict};
use serde::Deserialize;

use crate::{
    app::{App, blocking},
    charts,
    error::Result,
    filter::{Dim, Filter, FilterBar},
    fmt,
    routes::{Shared, with_records},
    views::{self, Chrome, Nav},
};

#[derive(Deserialize, Default)]
pub(crate) struct InsightsQuery {
    #[serde(flatten)]
    pub(crate) filter: Filter,
    /// `bots=1` counts bot accounts.
    bots: Option<String>,
    /// Comma separated globs left out of path statistics.
    exclude: Option<String>,
    /// `generated=1` counts lock files and minified output.
    generated: Option<String>,
    /// `ignore=0` counts paths that .gitignore rules out.
    ignore: Option<String>,
    /// `ws=1` counts whitespace-only changes.
    ws: Option<String>,
}

impl InsightsQuery {
    pub(crate) fn options(&self) -> Options {
        let on = |v: &Option<String>| v.as_deref() == Some("1");
        Options {
            exclude: self
                .exclude
                .as_deref()
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .map(str::to_owned)
                .collect(),
            include_generated: on(&self.generated),
            respect_ignore: self.ignore.as_deref() != Some("0"),
            count_whitespace: on(&self.ws),
            include_bots: on(&self.bots),
        }
    }

    /// Option parameters as they appear in a query, defaults left out.
    fn pairs(&self) -> Vec<(&'static str, String)> {
        let o = self.options();
        let mut v = Vec::new();
        if !o.exclude.is_empty() {
            v.push(("exclude", o.exclude.join(",")));
        }
        for (k, on) in [
            ("generated", o.include_generated),
            ("ws", o.count_whitespace),
            ("bots", o.include_bots),
        ] {
            if on {
                v.push((k, "1".to_owned()));
            }
        }
        if !o.respect_ignore {
            v.push(("ignore", "0".to_owned()));
        }
        v
    }

    /// Link to `base` with the current filters and options, `key` replaced by `value`
    /// (`None` removes it).
    fn link(&self, base: &str, key: &'static str, value: Option<&str>) -> String {
        let mut extra: Vec<(&str, String)> = self
            .pairs()
            .into_iter()
            .filter(|(k, _)| *k != key)
            .collect();
        if let Some(v) = value {
            extra.push((key, v.to_owned()));
        }
        let pairs: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
        fmt::href(&format!("{base}{}", self.filter.query(&pairs)))
    }

    /// The quiet options row under the filter bar.
    pub(crate) fn row(&self, base: &str) -> Vec<OptLink> {
        let o = self.options();
        let mut row = vec![
            OptLink::new(
                "Generated files",
                o.include_generated,
                "counted",
                "left out",
                self.link(base, "generated", (!o.include_generated).then_some("1")),
            ),
            OptLink::new(
                "Gitignored",
                !o.respect_ignore,
                "counted",
                "left out",
                self.link(base, "ignore", o.respect_ignore.then_some("0")),
            ),
            OptLink::new(
                "Whitespace",
                o.count_whitespace,
                "counted",
                "ignored",
                self.link(base, "ws", (!o.count_whitespace).then_some("1")),
            ),
            OptLink::new(
                "Bots",
                o.include_bots,
                "counted",
                "left out",
                self.link(base, "bots", (!o.include_bots).then_some("1")),
            ),
        ];
        if !o.exclude.is_empty() {
            row.push(OptLink {
                label: "Exclude",
                state: o.exclude.join(", "),
                href: self.link(base, "exclude", None),
                on: true,
            });
        }
        row
    }
}

pub(crate) struct OptLink {
    pub(crate) label: &'static str,
    pub(crate) state: String,
    pub(crate) href: String,
    pub(crate) on: bool,
}

impl OptLink {
    fn new(label: &'static str, on: bool, yes: &str, no: &str, href: String) -> Self {
        Self {
            label,
            state: (if on { yes } else { no }).to_owned(),
            href,
            on,
        }
    }
}

/// Insights over `recs` with the options in force. The head ignore rules apply when asked.
pub(crate) fn insights<'a>(a: &App, recs: &'a [CommitRecord], o: Options) -> Insights<'a> {
    let ignore = if o.respect_ignore {
        let head = a.repo.resolve("HEAD").unwrap_or_default();
        gitterrace_git::IgnoreSet::at(&a.repo, &head).ok()
    } else {
        None
    };
    Insights::new(recs, o, ignore)
}

/// Records after every filter except the period, which the insights methods window themselves.
pub(crate) fn scoped<'a>(
    f: &Filter,
    r: &'a [CommitRecord],
) -> std::borrow::Cow<'a, [CommitRecord]> {
    f.with(Dim::Period, None).apply(r)
}

/// Text files of HEAD with line counts, the cache persisted under `.git/gitterrace`.
pub(crate) fn head_lines(a: &App) -> Arc<Vec<gitterrace_index::HeadFile>> {
    a.head_lines(&a.repo.resolve("HEAD").unwrap_or_default())
}

/// Memoizes the rendered page per tip and query, building on a miss.
pub(crate) async fn cached<T: askama::Template + Send + 'static>(
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

pub(crate) fn period_of(q: &InsightsQuery, default: Period) -> Period {
    q.filter.period_value().unwrap_or(default)
}

pub(crate) struct Kpi(pub(crate) &'static str, pub(crate) String);

#[derive(Template)]
#[template(path = "code_frequency.html")]
struct CodeFrequencyPage {
    c: Chrome,
    tabs: Vec<Nav>,
    fb: FilterBar,
    opts: Vec<OptLink>,
    kpis: Vec<Kpi>,
    unit: &'static str,
    window: usize,
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
        let recs = scoped(f, r);
        let ins = insights(a, &recs, q.options());
        let period = period_of(&q, Period::All);
        let (unit, weeks) = ins.code_frequency(period);
        let (add, del, rew) = weeks.iter().fold((0, 0, 0), |t, w| {
            (t.0 + w.additions, t.1 + w.deletions, t.2 + w.rewritten)
        });
        let points: Vec<_> = weeks
            .iter()
            .map(|w| (w.label.clone(), w.additions, w.deletions))
            .collect();
        let rw = ins.rework(period);
        let verdict = rw.points.iter().rev().find_map(|p| p.share).map_or_else(
            || "No changes in this selection.".to_owned(),
            |s| {
                format!(
                    "Latest window of {} commits: {:.0}% of changed lines replaced existing code, {}.",
                    rw.window,
                    s * 100.0,
                    rework_verdict(s)
                )
            },
        );
        CodeFrequencyPage {
            c,
            tabs: views::stats_tabs("cf"),
            fb,
            opts: q.row("/stats/frequency"),
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
            window: rw.window,
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
    unit: &'static str,
    c: Chrome,
    tabs: Vec<Nav>,
    fb: FilterBar,
    opts: Vec<OptLink>,
    kpis: Vec<Kpi>,
    people: Vec<PersonRow>,
}

pub(crate) async fn contributors(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "co", raw, move |a, r, c| {
        let f = &q.filter;
        let bots = q.options().include_bots;
        let fb = f.bar("/stats/contributors", r);
        // The period is applied by Insights so first-time contributors are judged against all
        // history that the other filters keep.
        let recs = scoped(f, r);
        let ins = insights(a, &recs, q.options());
        let period = period_of(&q, Period::All);
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
                    spark: charts::sparkline(&p.series, "s1"),
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
        ContributorsPage {
            unit: report.unit.noun(),
            c,
            tabs: views::stats_tabs("co"),
            fb,
            opts: q.row("/stats/contributors"),
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
        }
    })
    .await
}
