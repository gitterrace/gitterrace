//! The review order panel of the working tree and compare pages: the changed files riskiest
//! first, each with the points behind its place. The numbers come from the API's review plan,
//! so the page, the JSON and the MCP tool agree. `]` and `[` walk the files in this order
//! (`nav.js`).

use askama::Template;
use gitterrace_api::{Params, PlanRequest};
use gitterrace_git::Scope;
use gitterrace_types::api::{PlanFile, PlanSkipped, ReviewPlan, SkipReason};

use crate::{app::App, charttip, fmt};

/// What the panel reads: the changed files of the working tree or of a comparison.
pub(crate) enum Source<'a> {
    Worktree { scope: Scope, since: Option<&'a str> },
    Compare { base: &'a str, head: &'a str, three_dot: bool },
}

struct Row {
    path: String,
    href: String,
    risk: u32,
    /// The four terms of the risk as "Churn 24 of 30" and so on.
    terms: Vec<String>,
    /// Plain-words reasons: rank, owner, size, test.
    why: Vec<String>,
}

struct Skipped {
    path: String,
    reason: &'static str,
}

#[derive(Template)]
#[template(path = "review_panel.html")]
struct Panel {
    summary: String,
    rows: Vec<Row>,
    skipped: Vec<Skipped>,
    more: String,
}

/// The panel for `source`, or `""` when there is nothing to order or the plan is unavailable
/// (insights off, index still building, a public server for the working tree).
pub(crate) fn render(a: &App, source: &Source<'_>) -> String {
    if !a.insights_open() {
        return String::new();
    }
    let Some(api) = a.api() else {
        return String::new();
    };
    let req = match *source {
        Source::Worktree { scope, since } => PlanRequest {
            source: Some("worktree"),
            scope: Some(scope_name(scope)),
            since,
            ..PlanRequest::default()
        },
        Source::Compare { base, head, three_dot } => PlanRequest {
            source: Some("compare"),
            base: Some(base),
            head: Some(head),
            three_dot: Some(three_dot),
            ..PlanRequest::default()
        },
    };
    let plan = match api.review_plan(&req, &Params::default()) {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("review panel: {e}");
            return String::new();
        }
    };
    if plan.files.is_empty() {
        return String::new();
    }
    let link = |path: &str| match source {
        Source::Worktree { .. } => format!("#{}", fmt::file_anchor(&fmt::enc_path(path))),
        Source::Compare { head, .. } => a
            .repo
            .resolve(head)
            .map_or_else(|_| String::new(), |id| fmt::commit_file_href(&id, path)),
    };
    panel(&plan, link).render().unwrap_or_default()
}

const fn scope_name(s: Scope) -> &'static str {
    match s {
        Scope::All => "all",
        Scope::Staged => "staged",
        Scope::Unstaged => "unstaged",
    }
}

fn panel(plan: &ReviewPlan, link: impl Fn(&str) -> String) -> Panel {
    Panel {
        summary: fmt::count(plan.total_files, "file", "files"),
        rows: plan
            .files
            .iter()
            .map(|f| Row {
                path: f.path.clone(),
                href: link(&f.path),
                risk: f.risk,
                terms: terms(f),
                why: why(f),
            })
            .collect(),
        skipped: plan.skipped.iter().map(skipped).collect(),
        more: if plan.truncated {
            fmt::count(
                plan.total_files - plan.files.len() - plan.skipped.len(),
                "more file",
                "more files",
            )
        } else {
            String::new()
        },
    }
}

fn terms(f: &PlanFile) -> Vec<String> {
    let t = f.terms;
    vec![
        format!("Churn {} of 30", t.churn),
        format!("Size {} of 25", t.size),
        format!("Ownership {} of 25", t.ownership),
        format!("Untested {} of 20", t.untested),
    ]
}

fn why(f: &PlanFile) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(h) = &f.hotspot {
        out.push(format!("Hotspot #{} of {}", fmt::thousands(h.rank), fmt::thousands(h.of)));
    }
    out.push(format!(
        "{} changed, {} long",
        fmt::count(f.additions + f.deletions, "line", "lines"),
        fmt::count(f.size_lines, "line", "lines")
    ));
    if let Some(o) = f.owners.first()
        && (f.sole_owner || f.bus_factor_one)
    {
        let lead = if f.sole_owner { "Sole owner" } else { "Bus factor one, led by" };
        out.push(format!("{lead} {} ({})", o.name, charttip::percent(o.share)));
    }
    if !f.is_test && !f.has_test_sibling {
        out.push("No test next to it".to_owned());
    }
    out
}

fn skipped(s: &PlanSkipped) -> Skipped {
    Skipped {
        path: s.path.clone(),
        reason: match s.reason {
            SkipReason::Lockfile => "lockfile",
            SkipReason::Generated => "generated",
            SkipReason::Binary => "binary",
            SkipReason::TooLarge => "too large to read",
        },
    }
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::Fixture;

    fn linked(p: &std::path::Path) -> axum::Router {
        let app = crate::tests::ready_app(p);
        app.set_api(gitterrace_api::ApiState::open(p).expect("api"));
        crate::routes::router(app)
    }

    /// The working tree page lists the changed files in the plan's order, with the points
    /// behind each, and the same order the API gives. Catches a panel that sorts on its own.
    #[tokio::test]
    async fn the_working_tree_page_lists_files_in_the_plan_order() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = Fixture::init(dir.path())
            .write("a.txt", "a\n")
            .write("b.txt", "b\n")
            .commit("feat: one")
            .path()
            .to_owned();
        std::fs::write(p.join("a.txt"), "a\n1\n").expect("write");
        std::fs::write(p.join("b.txt"), "b\n1\n2\n3\n4\n5\n6\n").expect("write");
        let r = linked(&p);
        let (st, page) = gitterrace_testkit::get(&r, "/worktree").await;
        assert_eq!(st, 200);
        let api = gitterrace_api::ApiState::open(&p).expect("api");
        let plan = api
            .review_plan(
                &gitterrace_api::PlanRequest::default(),
                &gitterrace_api::Params::default(),
            )
            .expect("plan");
        let order: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(order.len(), 2);
        let at: Vec<usize> = order
            .iter()
            .map(|n| page.find(&format!("data-rplan-path=\"{n}\"")).expect("row"))
            .collect();
        assert!(at[0] < at[1], "{order:?} {page}");
        assert!(page.contains("Churn "), "terms are inspectable");
    }
}
