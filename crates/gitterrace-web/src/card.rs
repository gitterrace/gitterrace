//! The file card: what one file or folder is in the history, as a fragment the blob, tree and
//! history pages fetch after their first paint. The numbers come from `file_insight`; this
//! module only words them. A cold index, a path the tree lacks or any failed read answers an
//! empty body, so the page keeps its layout and shows nothing.

use askama::Template;
use axum::{
    extract::{Path, State},
    response::Html,
};
use gitterrace_api::Params;
use gitterrace_config::Feature;
use gitterrace_types::api::{FileInsight, FileInsightKind};

use crate::{
    app::{App, blocking},
    charts::{self, view::Spark},
    charttip,
    error::Result,
    filter::{Dim, Filter},
    fmt,
    routes::Shared,
    when::When,
};

/// Owners and partners a card lists.
const SHOWN: usize = 3;

struct Partner {
    href: String,
    path: String,
    degree: String,
}

#[derive(Template)]
#[template(path = "insight_card.html")]
struct Card {
    hot: bool,
    rank: String,
    changes: String,
    lines: String,
    spark: Option<Spark>,
    owners: Vec<(String, String)>,
    bus: String,
    partners: Vec<Partner>,
    last: When,
    age: String,
    link: String,
}

struct Badge {
    name: String,
    href: String,
    title: String,
}

#[derive(Template)]
#[template(path = "insight_badges.html")]
struct Badges {
    rows: Vec<Badge>,
}

/// Where a page fetches the card of `path` at `rev`; empty when insights are off.
pub(crate) fn href(rev: &str, path: &str) -> String {
    let at = format!("/api/insight/{}", fmt::enc_seg(rev));
    if path.is_empty() {
        crate::features::href_if(Feature::Insights, &at)
    } else {
        crate::features::href_if(Feature::Insights, &format!("{at}/{}", fmt::enc_path(path)))
    }
}

/// `/api/insight/{rev}`: the card of the root folder.
pub(crate) async fn root(state: Shared, Path(rev): Path<String>) -> Result<Html<String>> {
    fragment(state, Path((rev, String::new()))).await
}

/// `/api/insight/{rev}/{path}`: a file's card, or the hot badges of a folder's entries.
pub(crate) async fn fragment(
    State(app): Shared,
    Path((rev, path)): Path<(String, String)>,
) -> Result<Html<String>> {
    let path = path.trim_matches('/').to_owned();
    let key = blocking(&app, {
        let (rev, path) = (rev.clone(), path.clone());
        move |a| {
            let id = a.repo.resolve(&rev)?;
            Ok(a.memo_key("card", Some(&format!("{rev}\0{id}\0{path}"))))
        }
    })
    .await?;
    if let Some(h) = app.memo_get(&key) {
        return Ok(Html(h));
    }
    let (html, served) = blocking(&app, move |a| Ok(render(a, &rev, &path))).await?;
    if served {
        app.memo_put(key, &html);
    }
    Ok(Html(html))
}

/// The fragment and whether it is final: an index still building answers empty and is not
/// remembered.
fn render(a: &App, rev: &str, path: &str) -> (String, bool) {
    if !a.insights_open() {
        return (String::new(), false);
    }
    let Some(api) = a.api() else {
        return (String::new(), false);
    };
    let found = match api.file_insight(Some(rev), path, &Params::default()) {
        Ok(f) => f,
        Err(e) => {
            tracing::debug!("file card for {path}: {e}");
            return (String::new(), false);
        }
    };
    let html = match found.kind {
        FileInsightKind::Missing => Ok(String::new()),
        FileInsightKind::File => card(&found).render(),
        FileInsightKind::Dir => badges(&found).render(),
    };
    match html {
        Ok(h) => (h, true),
        Err(e) => {
            tracing::warn!("file card for {path}: {e}");
            (String::new(), false)
        }
    }
}

fn stats_link(path: &str) -> String {
    Filter::default().link("/stats/hotspots", Dim::Path, path)
}

fn card(f: &FileInsight) -> Card {
    let now = fmt::now();
    let c = &f.churn;
    Card {
        hot: f.hot,
        rank: if c.rank > 0 {
            format!("#{} of {}", fmt::thousands(c.rank), fmt::thousands(c.of))
        } else {
            String::new()
        },
        changes: fmt::count(c.changes, "change", "changes"),
        lines: fmt::count(c.lines_changed, "line", "lines"),
        spark: (c.series.iter().filter(|v| **v > 0).count() > 1)
            .then(|| charts::sparkline(&c.series, "card")),
        owners: f
            .owners
            .iter()
            .take(SHOWN)
            .map(|o| (o.name.clone(), charttip::percent(o.share)))
            .collect(),
        bus: if f.bus_factor > 0 { fmt::thousands(f.bus_factor) } else { String::new() },
        partners: f
            .partners
            .iter()
            .take(SHOWN)
            .map(|p| Partner {
                href: fmt::file_link(&p.path),
                path: p.path.clone(),
                degree: format!(
                    "{} together, {}",
                    fmt::count(p.together, "commit", "commits"),
                    charttip::percent(p.degree)
                ),
            })
            .collect(),
        last: f.age.last_change.map_or_else(When::none, |t| fmt::ago(t, now)),
        age: f.age.median_line_age_secs.map(age_words).unwrap_or_default(),
        link: stats_link(&f.path),
    }
}

/// "3 months": how old half of the lines are.
fn age_words(secs: i64) -> String {
    let days = secs.max(0) / gitterrace_git::DAY;
    match days {
        0 => "under a day".to_owned(),
        1..=59 => fmt::count(days, "day", "days"),
        60..=729 => fmt::count(days / 30, "month", "months"),
        _ => fmt::count(days / 365, "year", "years"),
    }
}

fn badges(f: &FileInsight) -> Badges {
    Badges {
        rows: f
            .paths
            .iter()
            .filter(|c| c.hot)
            .map(|c| Badge {
                name: c.path.rsplit('/').next().unwrap_or_default().to_owned(),
                href: stats_link(&c.path),
                title: format!(
                    "Hotspot: rank {}, {}",
                    fmt::thousands(c.rank),
                    fmt::count(c.changes, "change", "changes")
                ),
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "card_tests.rs"]
mod tests;
