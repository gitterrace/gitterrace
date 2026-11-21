//! Insights pages over the shared filter: code frequency and contributors.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::feed::InsightsSource;
use gitterrace_index::feed::SystemClock;
use gitterrace_index::{
    Counted, Insights, Metric, OptionParams, Options, Period, Scope, ScopeParams,
};
use serde::Deserialize;

use crate::seg::{Choice, Seg};
use crate::{
    app::{App, blocking},
    charts,
    error::Result,
    filter::{Dim, Filter, FilterBar},
    fmt,
    pagehead::PageHead,
    routes::{Shared, with_records},
    views::{self, Chrome},
};

/// How long a request waits on a page the warm-up is rendering before it answers with the
/// pending page instead.
const WARM_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Deserialize, Default)]
pub(crate) struct InsightsQuery {
    #[serde(flatten)]
    pub(crate) filter: Filter,
    /// `bots`, `merges`, `generated`, `ignore`, `ws`, `exclude`: parsed as the API parses them.
    #[serde(flatten)]
    pub(crate) opts: OptionParams,
    /// `commits` (default), `added`, `deleted` or `net`: what the contributors list follows.
    pub(crate) metric: Option<String>,
}

/// What the options are when the URL says nothing: `insights.*` and `[repo.NAME]` of the config.
#[derive(Debug, Clone, Default)]
pub(crate) struct Defaults {
    pub(crate) bots: bool,
    pub(crate) generated: bool,
    /// Globs the config leaves out of every insight, always applied.
    pub(crate) exclude: Vec<String>,
    pub(crate) period: Option<Period>,
}

impl Defaults {
    /// Options for pages without option links (Compare, History).
    pub(crate) fn into_options(self) -> Options {
        Options {
            exclude: self.exclude,
            include_generated: self.generated,
            include_bots: self.bots,
            ..Options::default()
        }
    }
}

impl App {
    /// The counting options come from the function `/api/v1` and the MCP use, so a page and
    /// the API agree on what the config leaves out.
    pub(crate) fn defaults(&self) -> Defaults {
        let cfg = self.cfg();
        let o = gitterrace_api::options_from(&cfg, &self.name);
        Defaults {
            bots: o.include_bots,
            generated: o.include_generated,
            exclude: o.exclude,
            period: Some(cfg.insights.period).filter(|p| *p != Period::All),
        }
    }
}

impl InsightsQuery {
    fn own_exclude(&self) -> Vec<String> {
        self.opts.own_exclude()
    }

    pub(crate) fn metric(&self) -> Metric {
        self.metric.as_deref().map_or(Metric::Commits, Metric::parse)
    }

    pub(crate) fn options(&self, d: &Defaults) -> Options {
        self.opts.apply(d.clone().into_options())
    }

    fn pairs(&self, d: &Defaults) -> Vec<(&'static str, String)> {
        let o = self.options(d);
        let mut v = Vec::new();
        let own = self.own_exclude();
        if !own.is_empty() {
            v.push(("exclude", own.join(",")));
        }
        for (k, on, default) in [
            ("generated", o.include_generated, d.generated),
            ("ws", o.count_whitespace, false),
            ("bots", o.include_bots, d.bots),
            ("merges", o.include_merges, false),
        ] {
            if on != default {
                v.push((k, (if on { "1" } else { "0" }).to_owned()));
            }
        }
        if !o.respect_ignore {
            v.push(("ignore", "0".to_owned()));
        }
        if self.metric() != Metric::Commits {
            v.push(("metric", self.metric().key().to_owned()));
        }
        v
    }

    /// Link to `base` with the current filters and options, `key` replaced by `value`
    /// (`None` removes it).
    fn link(&self, d: &Defaults, base: &str, key: &'static str, value: Option<&str>) -> String {
        let mut extra: Vec<(&str, String)> =
            self.pairs(d).into_iter().filter(|(k, _)| *k != key).collect();
        if let Some(v) = value {
            extra.push((key, v.to_owned()));
        }
        let pairs: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
        fmt::href(&format!("{base}{}", self.filter.query(&pairs)))
    }

    /// Link to `base` with the current filters and options and the days `from` to `to`
    /// (`YYYY-MM-DD`, both inclusive) as the range, written as `?q=` like any filter.
    pub(crate) fn range_link(&self, d: &Defaults, base: &str, from: &str, to: &str) -> String {
        let filter = match gitterrace_index::parse_day(from).zip(gitterrace_index::parse_day(to)) {
            Some((a, b)) => self.filter.with_range(a, b),
            None => self.filter.clone(),
        };
        let pairs = self.pairs(d);
        let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        fmt::href(&format!("{base}{}", filter.query(&pairs)))
    }

    pub(crate) fn row(&self, d: &Defaults, base: &str) -> Vec<OptLink> {
        let o = self.options(d);
        // The link that flips an option: the opposite value, or none when that is the default.
        let flip = |key: &'static str, now: bool, default: bool| {
            let to = !now;
            self.link(d, base, key, (to != default).then_some(if to { "1" } else { "0" }))
        };
        let mut row = vec![
            OptLink::new(
                "Generated files",
                o.include_generated,
                "counted",
                "left out",
                flip("generated", o.include_generated, d.generated),
            ),
            OptLink::new(
                "Gitignored",
                !o.respect_ignore,
                "counted",
                "left out",
                self.link(d, base, "ignore", o.respect_ignore.then_some("0")),
            ),
            OptLink::new(
                "Whitespace",
                o.count_whitespace,
                "counted",
                "ignored",
                flip("ws", o.count_whitespace, false),
            ),
            OptLink::new(
                "Bots",
                o.include_bots,
                "counted",
                "left out",
                flip("bots", o.include_bots, d.bots),
            ),
            OptLink::new(
                "Merges",
                o.include_merges,
                "counted",
                "left out",
                flip("merges", o.include_merges, false),
            ),
        ];
        let own = self.own_exclude();
        if !own.is_empty() {
            row.push(OptLink {
                label: "Exclude",
                state: own.join(", "),
                href: self.link(d, base, "exclude", None),
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
        Self { label, state: (if on { yes } else { no }).to_owned(), href, on }
    }
}

/// What every insights page reads: the tip's source, or a filtered view over it.
pub(crate) type Src = Arc<dyn InsightsSource>;

/// The source as [`App::source`] hands it out: readable only through
/// [`gitterrace_index::feed::Checked::run`], which refuses a build that met a storage error.
pub(crate) type CheckedSrc = gitterrace_index::feed::Checked<dyn InsightsSource>;

/// Insights over `src` with the options in force. The head ignore rules apply when asked.
pub(crate) fn insights<'a>(a: &App, src: &'a Src, o: Options) -> Insights<'a> {
    let head = a.repo.resolve(&crate::fmt::selected()).unwrap_or_default();
    let respect = o.respect_ignore;
    // The full record set of the tip is what every unfiltered page reads: its derived tables
    // (path filter decisions, rename-followed touches, per-commit lines) are built once per
    // tip, index generation and options, not once per page. Filtered sets stay per request.
    let full = a.source(&head).ok().flatten().is_some_and(|tip| tip.holds(src));
    if full {
        shared_on(a, &**src, &head, o)
    } else {
        Insights::from_source(&**src, o, ignore_for(a, respect, &head))
    }
}

/// The shared tables of the full set of `head`, see [`insights`]. Any source of that set
/// will do: the overview's totals read it too.
pub(crate) fn shared_on<'a>(
    a: &App,
    src: &'a dyn gitterrace_index::feed::InsightsSource,
    head: &str,
    o: Options,
) -> Insights<'a> {
    let respect = o.respect_ignore;
    let key = tables_key(a, head);
    Insights::shared_source(&a.caches.tables, src, o, &key, || ignore_for(a, respect, head))
}

/// The ignore rules at `head`, when the options respect them.
pub(crate) fn ignore_for(a: &App, respect: bool, head: &str) -> Option<gitterrace_git::IgnoreSet> {
    respect.then(|| a.ignore_set(head)).flatten()
}

/// The key the full set of `head` keeps its shared tables under, the API's too.
pub(crate) fn tables_key(a: &App, head: &str) -> String {
    format!("{}\u{0}{head}\u{0}{}", a.base, a.index.generation())
}

/// Builds the tables every per-file page slices (path filter decisions, period anchors, the
/// tip index, the head tree's path ids) for the default options at `head`. The refresh runs
/// it before it marks the index ready, so the first page after ready finds them and never
/// races their build. They go with the shared tables when the server idles.
pub(crate) fn warm_shared(a: &App, head: &str) {
    let Ok(Some(src)) = a.source_before_ready(head) else {
        return;
    };
    let files = a.head_lines(head);
    drop(src.run(|s| {
        let ins = shared_on(a, &**s, head, a.defaults().into_options());
        ins.warm_files();
        ins.warm_paths(files.iter().map(|f| &*f.path));
        // The languages chain reads the whole first-parent chain once per head. What it
        // keeps is flat in history length. A walk that met a storage error is not kept.
        drop(a.chain(head, &**s, &crate::languages::path_filter(a, head)));
    }));
}

/// The page's [`Scope`]: its period, the URL's selection and options, through the constructor
/// the API uses too. Count it with [`counted`] over the whole source.
pub(crate) fn page_scope(q: &InsightsQuery, d: &Defaults, period: Period) -> Scope {
    let params = ScopeParams {
        period: Some(period.as_str().to_owned()),
        select: q.filter.selection(),
        options: q.opts.clone(),
        ..ScopeParams::default()
    };
    Scope::from_params(&params, d.clone().into_options(), period)
        .unwrap_or_else(|_| unreachable!("the period is parsed already"))
}

/// The commits `scope` counts in `ins`, anchored on the whole source `all`.
pub(crate) fn counted<'a>(ins: &'a Insights<'a>, scope: &'a Scope, all: &Src) -> Counted<'a> {
    ins.counted(scope, ins.context_for(&**all, &SystemClock))
}

/// Text files of HEAD with line counts, the cache persisted under `.git/gitterrace`.
pub(crate) fn head_lines(a: &App) -> Arc<Vec<gitterrace_index::HeadFile>> {
    a.head_lines(&a.repo.resolve(&crate::fmt::selected()).unwrap_or_default())
}

/// Memoizes the rendered page per tip and the parameters it reads, building on a miss.
pub(crate) async fn cached<T: askama::Template + Send + 'static>(
    app: &Arc<App>,
    tab: &'static str,
    raw: Option<String>,
    build: impl FnOnce(&App, &Src, Chrome) -> T + Send + 'static,
) -> Result<Html<String>> {
    let query = App::canonical_query(raw.as_deref());
    cached_keyed(app, tab, query, build).await
}

/// `cached` with a caller-built key, for pages keyed by more than their query.
pub(crate) async fn cached_keyed<T: askama::Template + Send + 'static>(
    app: &Arc<App>,
    tab: &'static str,
    query: String,
    build: impl FnOnce(&App, &Src, Chrome) -> T + Send + 'static,
) -> Result<Html<String>> {
    let key = blocking(app, move |a| Ok(a.memo_key(tab, Some(&query)))).await?;
    if let Some(h) = app.memo_get(&key) {
        return Ok(Html(h));
    }
    // One render per cold key: a request that waited here finds the page the first one put.
    let claim = app.render_lock(&key);
    let _held = match tokio::time::timeout(WARM_WAIT, claim.hold()).await {
        Ok(held) => held,
        // The warm-up holds the key and may take minutes on a large history: the visitor gets
        // the page that fills in once ready rather than waiting on it.
        Err(_) if app.state() == crate::app::RepoState::Warming => {
            return Ok(Html(blocking(app, move |a| crate::routes::pending(a, tab)).await?));
        }
        Err(_) => claim.hold().await,
    };
    if let Some(h) = app.memo_get(&key) {
        return Ok(Html(h));
    }
    #[cfg(test)]
    app.renders.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (html, served) = with_records(app, tab, build).await?;
    if served {
        app.memo_put(key, &html.0);
    }
    Ok(html)
}

/// The URL's period, else `insights.period` when set, else the page's own default.
pub(crate) fn period_of(q: &InsightsQuery, d: &Defaults, default: Period) -> Period {
    q.filter.period_value().or(d.period).unwrap_or(default)
}

#[derive(Debug)]
pub(crate) struct Kpi(pub(crate) &'static str, pub(crate) String);

impl Kpi {
    /// A bus factor of 1 is a risk: the figure takes the warning color, as in the table.
    pub(crate) fn warn(&self) -> bool {
        self.0 == "Bus factor" && self.1 == "1"
    }
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
    spark: crate::charts::view::Spark,
    hue: u32,
    initial: String,
    /// Share of the widest side, in percent: added to the right, deleted to the left.
    bar_add: String,
    bar_del: String,
}

#[derive(Template)]
#[template(path = "contributors.html")]
struct ContributorsPage {
    unit: &'static str,
    c: Chrome,
    head: PageHead,
    fb: FilterBar,
    kpis: Vec<Kpi>,
    people: Vec<PersonRow>,
    /// "Show all N contributors" when the list was cut at `PEOPLE_SHOWN`.
    more: Option<(String, String)>,
    /// True for any metric but commits: the two-sided bar shows.
    alt: bool,
    hint: String,
}

/// Rows the contributors page renders unless asked for all: one row carries an inline
/// sparkline, so a large project's full list runs to megabytes. The API keeps every row.
pub(crate) const PEOPLE_SHOWN: usize = 100;

pub(crate) async fn contributors(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    let all = raw.as_deref().unwrap_or_default().split('&').any(|p| p == "all=1");
    cached(&app, "co", raw, move |a, r, c| {
        let f = &q.filter;
        let d = a.defaults();
        let period = period_of(&q, &d, Period::All);
        let mut fb = f.bar("/stats/contributors", &**r, period);
        fb.extra = Some(q.metric_switch(&d));
        // The period is applied by Insights so first-time contributors are judged against all
        // history that the options keep, whatever the selection.
        let scope = page_scope(&q, &d, period);
        let ins = insights(a, r, scope.options().clone());
        let metric = q.metric();
        let report = counted(&ins, &scope, r).contributors(metric);
        let shown: Vec<_> = report.contributors.iter().collect();
        let total = shown.len();
        let new_total = shown.iter().filter(|p| p.first_time).count();
        let more = (!all && total > PEOPLE_SHOWN).then(|| {
            (
                q.link(&d, "/stats/contributors", "all", Some("1")),
                format!("Show all {}", fmt::count(total, "contributor", "contributors")),
            )
        });
        let shown_rows = shown.len().min(if all { usize::MAX } else { PEOPLE_SHOWN });
        let widest = shown
            .iter()
            .take(shown_rows)
            .map(|p| p.additions.max(p.deletions))
            .max()
            .unwrap_or(0)
            .max(1);
        let pct = |v: u64| format!("{}", (v as f64 * 100.0 / widest as f64).round());
        let people: Vec<PersonRow> = shown
            .iter()
            .take(if all { usize::MAX } else { PEOPLE_SHOWN })
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
                    initial: views::initials(&p.name),
                    bar_add: pct(p.additions),
                    bar_del: pct(p.deletions),
                }
            })
            .collect();
        let (add, del): (u64, u64) =
            report.contributors.iter().fold((0, 0), |t, p| (t.0 + p.additions, t.1 + p.deletions));
        ContributorsPage {
            unit: report.unit.noun(),
            c,
            head: crate::pagehead::insights_head(
                "co",
                crate::pagehead::lead_tagline(&crate::lead::contributors(total, new_total)),
                fb.toolbar(&q.row(&d, "/stats/contributors")),
            ),
            fb,
            kpis: vec![
                Kpi("Contributors", fmt::thousands(total)),
                Kpi("New in period", fmt::thousands(new_total)),
                Kpi("Lines added", fmt::thousands(add)),
                Kpi("Lines removed", fmt::thousands(del)),
            ],
            people,
            more,
            alt: metric != Metric::Commits,
            hint: metric_hint(metric).to_owned(),
        }
    })
    .await
}

/// The sentence start the page shows above the list for a non-default metric.
const fn metric_hint(m: Metric) -> &'static str {
    match m {
        Metric::Commits => "",
        Metric::Added => "Ordered by lines added. The sparkline is lines added",
        Metric::Deleted => "Ordered by lines deleted. The sparkline is lines deleted",
        Metric::Net => {
            "Ordered by lines added minus lines deleted. The sparkline is that net, never below \
            zero"
        }
    }
}

impl InsightsQuery {
    /// One choice per metric, the one in force marked.
    fn metric_switch(&self, d: &Defaults) -> Seg {
        let choices = [
            (Metric::Commits, "Commits"),
            (Metric::Added, "Added"),
            (Metric::Deleted, "Deleted"),
            (Metric::Net, "Net"),
        ]
        .into_iter()
        .map(|(m, label)| {
            Choice::new(
                label,
                self.link(
                    d,
                    "/stats/contributors",
                    "metric",
                    (m != Metric::Commits).then(|| m.key()),
                ),
                m == self.metric(),
            )
        })
        .collect();
        Seg::links("Metric", choices).no_caption()
    }
}

#[cfg(test)]
#[path = "insights_tests.rs"]
mod tests;
