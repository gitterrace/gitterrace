//! The patterns sections of the hotspots page: files that change together, code age, commit
//! sizes and release cadence.

use askama::Template;
use gitterrace_index::Period;

use crate::{
    charts,
    empty::Empty,
    fmt::{self, enc_seg, href},
    insights::{InsightsQuery, Src, counted, head_lines, insights, page_scope, period_of},
    views::{self},
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
#[template(path = "patterns_section.html")]
struct PatternsSection {
    pairs: Vec<PairRow>,
    age: String,
    sizes: String,
    releases: Vec<ReleaseRow>,
    cadence: String,
    /// Shown in place of the releases table when no tag reads as a release.
    no_releases: Option<Empty>,
}

/// The sections' markup for the commits `q` selects.
pub(crate) fn section(a: &crate::app::App, r: &Src, q: &InsightsQuery) -> String {
    {
        let defaults = a.defaults();
        let period = period_of(q, &defaults, Period::All);
        let scope = page_scope(q, &defaults, period);
        let whole = insights(a, r, scope.options().clone());
        let set = counted(&whole, &scope, r);
        let files = head_lines(a);
        let pairs = whole
            .coupling_in(&files, &set, 3, TOP_PAIRS)
            .into_iter()
            .map(|p| PairRow {
                a_href: views::file_link(&p.a),
                b_href: views::file_link(&p.b),
                a: fmt::path_breaks(&p.a),
                b: fmt::path_breaks(&p.b),
                together: fmt::thousands(p.together),
                degree: format!("{:.0}%", p.degree * 100.0),
            })
            .collect();
        // Code age and release cadence are whole-history views: the same selection and
        // options over every day, counted by the same rule as the rest of the page.
        let always = page_scope(q, &defaults, Period::All);
        let ever = counted(&whole, &always, r);
        let age: Vec<_> = whole
            .code_age_in(&files, &ever)
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
        let cad = whole.cadence_in(&tags, &ever);
        let cadence = match (cad.median_days_between, cad.days_since_last) {
            (Some(median), Some(since)) => {
                format!(
                    "Median {} between releases, {} since the last.",
                    fmt::count(median.round() as i64, "day", "days"),
                    fmt::count(since, "day", "days")
                )
            }
            (None, Some(d)) => format!("{} since the last release.", fmt::count(d, "day", "days")),
            _ => String::new(),
        };
        let releases: Vec<ReleaseRow> = cad
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
        if set.stream().next().is_none() {
            return String::new();
        }
        PatternsSection {
            pairs,
            age: charts::bars(&age, crate::charttip::LINES),
            sizes: charts::bars(&sizes, crate::charttip::COMMITS),
            no_releases: releases.is_empty().then(|| {
                Empty::new("No release tags.").step("See all branches and tags", href("/refs"))
            }),
            releases,
            cadence,
        }
        .render()
        .unwrap_or_default()
    }
}
