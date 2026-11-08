//! Insights pages over the shared filter: code frequency and contributors.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::feed::InsightsSource;
use gitterrace_index::feed::SystemClock;
use gitterrace_index::{
    CountContext, Counted, Insights, Metric, OptionParams, Options, Period, Scope, ScopeParams,
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
    /// `None` leaves each page its own default period.
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
        self.metric
            .as_deref()
            .map_or(Metric::Commits, Metric::parse)
    }

    pub(crate) fn options(&self, d: &Defaults) -> Options {
        self.opts.apply(d.clone().into_options())
    }

    /// Option parameters as they appear in a query, defaults left out.
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
        let mut extra: Vec<(&str, String)> = self
            .pairs(d)
            .into_iter()
            .filter(|(k, _)| *k != key)
            .collect();
        if let Some(v) = value {
            extra.push((key, v.to_owned()));
        }
        let pairs: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
        fmt::href(&format!("{base}{}", self.filter.query(&pairs)))
    }

    /// Link to `base` with the current filters and options and the date range `from` to `to`.
    pub(crate) fn unranged_link(&self, d: &Defaults, base: &str) -> String {
        self.link(d, base, "from", None)
    }

    pub(crate) fn range_link(&self, d: &Defaults, base: &str, from: &str, to: &str) -> String {
        let mut extra = self.pairs(d);
        extra.push(("from", from.to_owned()));
        extra.push(("to", to.to_owned()));
        let pairs: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
        fmt::href(&format!("{base}{}", self.filter.query(&pairs)))
    }

    /// The quiet options row under the filter bar.
    pub(crate) fn row(&self, d: &Defaults, base: &str) -> Vec<OptLink> {
        let o = self.options(d);
        // The link that flips an option: the opposite value, or none when that is the default.
        let flip = |key: &'static str, now: bool, default: bool| {
            let to = !now;
            self.link(
                d,
                base,
                key,
                (to != default).then_some(if to { "1" } else { "0" }),
            )
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
        Self {
            label,
            state: (if on { yes } else { no }).to_owned(),
            href,
            on,
        }
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
    let ignore = |a: &App| respect.then(|| a.ignore_set(&head)).flatten();
    // The full record set of the tip is what every unfiltered page reads: its derived tables
    // (path filter decisions, rename-followed touches, per-commit lines) are built once per
    // tip, index generation and options, not once per page. Filtered sets stay per request.
    let full = a
        .source(&head)
        .ok()
        .flatten()
        .is_some_and(|tip| tip.holds(src));
    if full {
        let key = format!("{}\u{0}{head}\u{0}{}", a.base, a.index.generation());
        Insights::shared_source(&**src, o, &key, || ignore(a))
    } else {
        Insights::from_source(&**src, o, ignore(a))
    }
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
    ins.counted(scope, CountContext::new(&**all, &SystemClock))
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
    let (html, served) = with_records(app, tab, build).await?;
    if served {
        app.memo_put(key, Arc::new(html.0.clone()));
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
    let all = raw
        .as_deref()
        .unwrap_or_default()
        .split('&')
        .any(|p| p == "all=1");
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
                format!(
                    "Show all {}",
                    fmt::count(total, "contributor", "contributors")
                ),
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
        let (add, del): (u64, u64) = report
            .contributors
            .iter()
            .fold((0, 0), |t, p| (t.0 + p.additions, t.1 + p.deletions));
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
mod tests {
    use std::sync::Arc;

    use askama::Template;

    use crate::app::App;

    #[derive(Template)]
    #[template(source = "data", ext = "txt")]
    struct Data;

    /// The indexing page rendered just before the index turns ready is never memoized: the
    /// next request after ready shows data, not a stale indexing page for the memo's minute.
    #[tokio::test]
    async fn the_indexing_page_is_never_memoized() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = gitterrace_testkit::Fixture::init(dir.path())
            .write("a.txt", "a\n")
            .commit("feat: a")
            .path()
            .to_owned();
        let repo = gitterrace_git::Repo::discover(&p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        // The build has finished (the index generation moved) but the ready flag is not set
        // yet: a request in that window renders the indexing page under the final memo key.
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        let first = super::cached(&app, "a", None, |_, _, _| Data)
            .await
            .expect("page");
        assert!(first.0.contains("data-progress"), "{}", first.0);
        app.mark_ready();
        let second = super::cached(&app, "a", None, |_, _, _| Data)
            .await
            .expect("page");
        assert_eq!(second.0, "data");
    }
}
