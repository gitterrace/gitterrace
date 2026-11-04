//! The patterns page, `/stats/patterns`: files that change together, ages, sizes and release cadence.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::Period;

use crate::{
    charts,
    error::Result,
    filter::FilterBar,
    fmt::{self, enc_seg, href},
    insights::{
        InsightsQuery, OptLink, cached, counted, head_lines, insights, page_scope, period_of,
        scoped,
    },
    routes::Shared,
    views::{self, Chrome, Tabs},
};

const TOP_PAIRS: usize = 30;

struct PairRow {
    a: String,
    a_href: String,
    b: String,
    b_href: String,
    together: String,
    degree: String,
}

struct ReleaseRow {
    name: String,
    href: String,
    when: crate::when::When,
    commits: String,
    days: String,
}

#[derive(Template)]
#[template(path = "patterns.html")]
struct PatternsPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    empty: bool,
    pairs: Vec<PairRow>,
    age: String,
    sizes: String,
    releases: Vec<ReleaseRow>,
    cadence: String,
}

pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "pa", raw, move |a, r, c| {
        let f = &q.filter;
        let d = a.defaults();
        let period = period_of(&q, &d, Period::All);
        let scope = page_scope(&q, &d, period);
        let whole = insights(a, r, scope.options().clone());
        let set = counted(&whole, &scope, r);
        let files = head_lines(a);
        let pairs = whole
            .coupling_in(&files, &set, 3, TOP_PAIRS)
            .into_iter()
            .map(|p| PairRow {
                a_href: views::file_link(&p.a),
                b_href: views::file_link(&p.b),
                a: p.a,
                b: p.b,
                together: fmt::thousands(p.together),
                degree: format!("{:.0}%", p.degree * 100.0),
            })
            .collect();
        // Code age and release cadence cover all history: no Counted reader yet.
        let recs = scoped(f, r);
        let ins = insights(a, &recs, q.options(&d));
        let age: Vec<_> = ins
            .code_age(&files)
            .buckets
            .into_iter()
            .map(|b| (b.year.to_string(), b.lines))
            .collect();
        let sizes: Vec<_> = set
            .sizes()
            .into_iter()
            .map(|b| (b.label, b.commits as u64))
            .collect();
        let tags = gitterrace_index::release_tags(&a.repo.refs().unwrap_or_default());
        let cad = ins.cadence(&tags);
        let cadence = match (cad.median_days_between, cad.days_since_last) {
            (Some(m), Some(d)) => {
                format!(
                    "Median {} between releases, {} since the last.",
                    fmt::count(m.round() as i64, "day", "days"),
                    fmt::count(d, "day", "days")
                )
            }
            (None, Some(d)) => format!("{} since the last release.", fmt::count(d, "day", "days")),
            _ => "No release tags.".to_owned(),
        };
        let releases = cad
            .releases
            .iter()
            .rev()
            .take(15)
            .map(|rl| ReleaseRow {
                href: href(&format!("/log/{}", enc_seg(&rl.name))),
                name: rl.name.clone(),
                when: crate::when::When::new(fmt::day(rl.time), rl.time, 0),
                commits: fmt::thousands(rl.commits_since),
                days: rl
                    .days_since
                    .map_or_else(|| "-".to_owned(), |d| d.to_string()),
            })
            .collect();
        PatternsPage {
            c,
            tabs: views::stats_tabs("pa"),
            fb: f.bar("/stats/patterns", &**r),
            opts: q.row(&d, "/stats/patterns"),
            empty: !ins.has_commits(period),
            pairs,
            age: charts::bars(&age),
            sizes: charts::bars(&sizes),
            releases,
            cadence,
        }
    })
    .await
}
