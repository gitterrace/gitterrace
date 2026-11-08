//! The ownership page, `/stats/ownership`: knowledge share and bus factor per directory.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::Period;

use crate::{
    error::Result,
    filter::{Dim, FilterBar},
    fmt::{self},
    insights::{InsightsQuery, Kpi, cached, counted, head_lines, insights, page_scope, period_of},
    pagehead::PageHead,
    routes::Shared,
    views::Chrome,
};

const TOP_DIRS: usize = 40;

struct DirRow {
    /// The directory split at `/`, for the `_path.html` macro.
    path: Vec<String>,
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
    head: PageHead,
    fb: FilterBar,
    kpis: Vec<Kpi>,
    rows: Vec<DirRow>,
    /// The one person who owns every directory, when that is so: the table then leaves out the
    /// Owners and Bus factor columns, which would repeat that name and a 1 on every row.
    solo: Option<String>,
}

/// The single owner every row shares, if each row lists exactly one owner and it is the same
/// person throughout.
fn sole_owner(rows: &[DirRow]) -> Option<String> {
    let first = rows.first()?.owners.first()?.0.clone();
    rows.iter()
        .all(|r| r.owners.len() == 1 && r.owners[0].0 == first)
        .then_some(first)
}

/// The repository root's own files come keyed by an empty path: name them.
fn dir_label(path: &str) -> String {
    if path.is_empty() {
        "(root)".to_owned()
    } else {
        path.to_owned()
    }
}

/// An owner whose share rounds to 0% holds nothing the table can show, and is not counted.
fn shown(o: &gitterrace_index::Owner) -> bool {
    o.share * 100.0 >= 0.5
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
            path: fmt::path_parts(&name),
            lines: fmt::thousands(d.lines),
            files: fmt::thousands(d.files),
            owners: d
                .owners
                .iter()
                .filter(|o| shown(o))
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
        let solo = sole_owner(&rows);
        let fb = f.bar("/stats/ownership", &**r, period_of(&q, &d, Period::All));
        OwnershipPage {
            solo,
            c,
            head: crate::pagehead::insights_head(
                "ow",
                crate::pagehead::lead_tagline(&crate::lead::ownership(
                    rep.overall.bus_factor,
                    rep.overall.owners.iter().filter(|o| shown(o)).count(),
                    rep.dirs_with_bus_factor_one,
                )),
                fb.toolbar(&q.row(&d, "/stats/ownership")),
            ),
            fb,
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
    use super::{DirRow, dir_label, sole_owner};

    fn row(owners: &[&str]) -> DirRow {
        DirRow {
            path: Vec::new(),
            href: String::new(),
            lines: String::new(),
            files: String::new(),
            owners: owners
                .iter()
                .map(|o| ((*o).to_owned(), "100%".to_owned()))
                .collect(),
            bus: 1,
        }
    }

    #[test]
    fn one_owner_everywhere_is_named_once() {
        assert_eq!(
            sole_owner(&[row(&["Ann"]), row(&["Ann"])]).as_deref(),
            Some("Ann")
        );
    }

    #[test]
    fn varied_ownership_keeps_the_columns() {
        assert_eq!(sole_owner(&[row(&["Ann"]), row(&["Bob"])]), None);
        assert_eq!(sole_owner(&[row(&["Ann"]), row(&["Ann", "Bob"])]), None);
        assert_eq!(sole_owner(&[row(&[])]), None);
    }

    #[test]
    fn root_files_get_a_label() {
        assert_eq!(dir_label(""), "(root)");
        assert_eq!(dir_label("src"), "src");
    }
}
