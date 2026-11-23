//! The patterns sections of the hotspots page: files that change together, code age, commit
//! sizes and release cadence.

use askama::Template;
use gitterrace_index::Period;

use crate::{
    charts,
    empty::Empty,
    fmt::{self, enc_seg, href},
    insights::{InsightsQuery, Src, counted, head_lines, insights, page_scope},
};

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
    coupled: String,
    age: crate::charts::view::Bars,
    sizes: crate::charts::view::Bars,
    releases: Vec<ReleaseRow>,
    cadence: String,
    /// Shown in place of the releases table when no tag reads as a release.
    no_releases: Option<Empty>,
}

pub(crate) fn section(
    a: &crate::app::App,
    r: &Src,
    q: &InsightsQuery,
    coupling: &crate::coupling::CouplingParams,
) -> String {
    let defaults = a.defaults();
    let (_, scope) = q.scoped(&defaults, Period::All);
    let whole = insights(a, r, scope.options().clone());
    let set = counted(&whole, &scope, r);
    // Nothing counted: skip the coupling, age and release reads below.
    if set.stream().next().is_none() {
        return String::new();
    }
    let files = head_lines(a);
    let coupled = crate::coupling::section(&whole, &set, &files, q, &defaults, coupling);
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
    let sizes: Vec<_> = set.sizes().into_iter().map(|b| (b.label, b.commits as u64)).collect();
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
            days: rl.days_since.map_or_else(|| "-".to_owned(), |d| d.to_string()),
        })
        .collect();
    PatternsSection {
        coupled,
        age: charts::bars("Code age", &age, crate::charttip::LINES),
        sizes: charts::bars("Commit sizes", &sizes, crate::charttip::COMMITS),
        no_releases: releases.is_empty().then(|| {
            Empty::new("No release tags.").step("See all branches and tags", href("/refs"))
        }),
        releases,
        cadence,
    }
    .render()
    .unwrap_or_default()
}
