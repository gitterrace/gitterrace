//! The ownership page, `/stats/ownership`: knowledge share and bus factor per directory.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::Period;

use crate::{
    error::Result,
    filter::{Dim, FilterBar},
    fmt::{self},
    insights::{
        InsightsQuery, Kpi, OptLink, cached, counted, head_lines, insights, page_scope, period_of,
    },
    routes::Shared,
    views::{self, Chrome, Tabs},
};

const TOP_DIRS: usize = 40;

struct DirRow {
    path: String,
    href: String,
    lines: String,
    files: String,
    owners: Vec<(String, String)>,
    bus: usize,
}

#[derive(Template)]
#[template(path = "ownership.html")]
struct OwnershipPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    kpis: Vec<Kpi>,
    rows: Vec<DirRow>,
}

/// The repository root's own files come keyed by an empty path: name them.
fn dir_label(path: &str) -> String {
    if path.is_empty() {
        "(root)".to_owned()
    } else {
        path.to_owned()
    }
}

pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "ow", raw, move |a, r, c| {
        let f = &q.filter;
        let d = a.defaults();
        let scope = page_scope(&q, &d, period_of(&q, &d, Period::All));
        let ins = insights(a, r, scope.options().clone());
        let files = head_lines(a);
        let rep = ins.ownership_in(
            &files,
            &counted(&ins, &scope, r),
            gitterrace_api::OWNERSHIP_DEPTH,
        );
        let row = |d: &gitterrace_index::DirOwnership, name: String| DirRow {
            href: f.link("/stats/ownership", Dim::Path, &d.path),
            path: name,
            lines: fmt::thousands(d.lines),
            files: fmt::thousands(d.files),
            owners: d
                .owners
                .iter()
                .take(3)
                .map(|o| (o.name.clone(), format!("{:.0}%", o.share * 100.0)))
                .collect(),
            bus: d.bus_factor,
        };
        let mut rows = vec![row(&rep.overall, "(whole repository)".to_owned())];
        rows.extend(
            rep.dirs
                .iter()
                .take(TOP_DIRS)
                .map(|d| row(d, dir_label(&d.path))),
        );
        OwnershipPage {
            c,
            tabs: views::stats_tabs("ow"),
            fb: f.bar("/stats/ownership", &**r),
            opts: q.row(&d, "/stats/ownership"),
            kpis: vec![
                Kpi("Bus factor", fmt::thousands(rep.overall.bus_factor)),
                Kpi("Solo lines", fmt::thousands(rep.solo_lines)),
                Kpi("Stale lines", fmt::thousands(rep.stale_lines)),
                Kpi(
                    "Directories at one",
                    fmt::thousands(rep.dirs_with_bus_factor_one),
                ),
            ],
            rows,
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::dir_label;

    #[test]
    fn root_files_get_a_label() {
        assert_eq!(dir_label(""), "(root)");
        assert_eq!(dir_label("src"), "src");
    }
}
