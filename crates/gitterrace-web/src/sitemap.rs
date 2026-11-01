//! The registry of what the server answers, outside `/api/v1` (that has its own in
//! `gitterrace-api`). The architecture page renders it, and the test below fails when a
//! `.route(...)` is added or removed without an entry here.

/// One row of the route table.
pub struct Page {
    /// What it is, as shown in the table's first column.
    pub shown: &'static str,
    pub does: &'static str,
    /// Every route literal this row covers, exactly as written in the routers.
    pub routes: &'static [&'static str],
}

const fn page(shown: &'static str, does: &'static str, routes: &'static [&'static str]) -> Page {
    Page {
        shown,
        does,
        routes,
    }
}

/// Every route a repository serves, under `/` or `/r/{name}/`, then the ones outside it.
pub const PAGES: &[Page] = &[
    page("`/`", "README, last commit, file list, key numbers", &["/"]),
    page(
        "`/tree/{rev}/{path}`",
        "Directory listing",
        &["/tree/{rev}", "/tree/{rev}/", "/tree/{rev}/{*path}"],
    ),
    page(
        "`/blob/{rev}/{path}`",
        "File with highlighting; large files load in chunks",
        &["/blob/{rev}/{*path}", "/hl/{rev}/{*path}"],
    ),
    page(
        "`/raw/{rev}/{path}`",
        "The bytes; over `limits.raw_mb` is refused",
        &["/raw/{rev}/{*path}"],
    ),
    page(
        "`/blame/{rev}/{path}`",
        "Line authorship",
        &["/blame/{rev}/{*path}"],
    ),
    page(
        "`/log/{rev}`",
        "Commits, filterable by author",
        &["/log", "/log/{rev}"],
    ),
    page(
        "`/history/{path}`",
        "One file's commits with lines changed and renames",
        &["/history/{*path}"],
    ),
    page(
        "`/commit/{id}`, `/diff/{rev}/{index}`",
        "A commit's diff, large files fold and load on demand",
        &["/commit/{id}", "/diff/{rev}/{index}"],
    ),
    page(
        "`/compare?base=&head=`",
        "What changed between two revisions",
        &["/compare"],
    ),
    page(
        "`/refs`, `/tag/{name}`",
        "Branches with divergence, tags with annotations",
        &["/refs", "/tag/{name}"],
    ),
    page(
        "`/worktrees`, `/stashes`",
        "Linked worktrees and stashes",
        &["/worktrees", "/stashes"],
    ),
    page(
        "`/worktree`",
        "Uncommitted changes, read-only",
        &["/worktree"],
    ),
    page("`/search`", "Code and commit search", &["/search"]),
    page(
        "`/stats`, `/stats/{page}`",
        "Insights on the shared filter and period. `?rev=` follows any ref",
        &[
            "/stats",
            "/stats/files",
            "/stats/types",
            "/stats/languages",
            "/stats/frequency",
            "/stats/pulse",
            "/stats/hotspots",
            "/stats/ownership",
            "/stats/patterns",
            "/stats/contributors",
        ],
    ),
    page(
        "`/badges/{name}`, `/cards/wrapped/{year}`",
        "SVG badges and the year card",
        &["/badges/{name}", "/cards/wrapped/{year}"],
    ),
    page(
        "`/events`",
        "Server-sent events: the page refreshes when the repository changes",
        &["/events"],
    ),
    page(
        "`/api/files/{rev}`, `/api/refs`, `/api/last/{rev}/{path}`, `/api/progress`",
        "Fragments the pages fetch (file finder, ref switcher, last commits, index progress)",
        &[
            "/api/files/{rev}",
            "/api/refs",
            "/api/last/{rev}",
            "/api/last/{rev}/{*path}",
            "/api/progress",
        ],
    ),
    page("`/api/v1/...`", "JSON API, see [api.md](api.md)", &[]),
    page(
        "`/go/{repo-id}/{sha}/{path}`",
        "Permalink: finds the served repository by id and redirects",
        &["/go/{*rest}"],
    ),
    page(
        "`/r/{name}/...`",
        "Everything above for one repository, under several",
        &["/r/{name}", "/r/{name}/", "/r/{name}/{*rest}"],
    ),
    page(
        "`/api/repos`",
        "The repository list the switcher reads",
        &["/api/repos"],
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::sitemap::PAGES;

    /// Every `.route("<literal>"` in a router source file.
    fn literals(src: &str) -> BTreeSet<String> {
        src.split(".route(")
            .skip(1)
            .filter_map(|rest| rest.trim_start().strip_prefix('"'))
            .filter_map(|rest| rest.split('"').next())
            .filter(|r| !r.starts_with("/static") && !r.starts_with("/_actions"))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn every_route_is_documented_and_every_entry_is_a_route() {
        let mut routed = BTreeSet::new();
        for src in [
            include_str!("routes.rs"),
            include_str!("multi.rs"),
            include_str!("actions.rs"),
        ] {
            routed.extend(literals(src));
        }
        let documented: BTreeSet<String> = PAGES
            .iter()
            .flat_map(|p| p.routes.iter().map(|r| (*r).to_owned()))
            .collect();
        assert_eq!(
            routed, documented,
            "a route and the PAGES registry disagree: add or remove its entry in sitemap.rs"
        );
    }
}
