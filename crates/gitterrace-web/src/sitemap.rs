//! The registry of what the server answers, outside `/api/v1`.
//!
//! (That has its own in `gitterrace-api`.) The architecture page renders it, and
//! `security_class_tests` fails when the built routers hold a route without an entry here.

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

/// Every route a repository serves, under its `/r/{name}/`, then the ones outside it.
pub const PAGES: &[Page] = &[
    page(
        "`/`",
        "README, last commit, file list, key numbers. The server's own `/` forwards to the one repository, or lists several",
        &["/"],
    ),
    page(
        "`/tree`, `/blob`, `/raw`, `/blame` `/{rev}/{path}`",
        "Files: listing, highlighted view (large files load in chunks), bytes, authorship",
        &[
            "/tree/{rev}",
            "/tree/{rev}/",
            "/tree/{rev}/{*path}",
            "/blob/{rev}/{*path}",
            "/hl/{rev}/{*path}",
            "/raw/{rev}/{*path}",
            "/blame/{rev}/{*path}",
        ],
    ),
    page(
        "`/log/{rev}`, `/history/{path}`, `/compare`",
        "Commits, one file's history, two revisions",
        &["/log", "/log/{rev}", "/history/{*path}", "/compare"],
    ),
    page(
        "`/commit/{id}`, `/diff/{rev}/{index}`",
        "A commit's diff, large files fold and load on demand",
        &["/commit/{id}", "/diff/{rev}/{index}"],
    ),
    page(
        concat!(
            "`/refs`, `/tag/{name}`, `/worktrees`, `/stashes`, `/worktree`, ",
            "`/worktree/diff/{path}`, `/worktree/raw/{path}`",
        ),
        concat!(
            "Git state: branches, tags, linked worktrees, stashes, uncommitted changes, ",
            "their folded files and changed images",
        ),
        &[
            "/refs",
            "/tag/{name}",
            "/worktrees",
            "/stashes",
            "/worktree",
            "/worktree/diff/{*path}",
            "/worktree/raw/{*path}",
        ],
    ),
    page("`/search`", "Code and commit search", &["/search"]),
    page(
        "`/stats`, `/stats/{page}`",
        concat!(
            "Insights on the shared filter and period: activity (with commit types and code ",
            "frequency), contributors, hotspots (with patterns), languages, ownership. ",
            "`/stats/types`, `/stats/frequency` and `/stats/patterns` answer with a permanent ",
            "redirect to their section, query kept, and `/insights` to `/stats`. ",
            "`?rev=` follows any ref",
        ),
        &[
            "/stats",
            "/stats/types",
            "/stats/languages",
            "/stats/frequency",
            "/stats/hotspots",
            "/stats/ownership",
            "/stats/patterns",
            "/stats/contributors",
            "/insights",
        ],
    ),
    page(
        "`/events`, `/api/{files,refs,last,progress}`",
        "Live updates and the fragments pages fetch",
        &[
            "/events",
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
        "Permalink: finds the served repository by id",
        &["/go/{*rest}"],
    ),
    page(
        "`/_send/targets`",
        "Send to targets as JSON template parts, filled in by the page",
        &["/_send/targets", "/r/{name}/_send/targets"],
    ),
    page(
        "`/r/{name}/...`, `/api/repos`",
        "The prefix of every per-repository row above, also when one is served; and the server-wide list the switcher reads",
        &["/r/{name}", "/r/{name}/", "/r/{name}/{*rest}", "/api/repos"],
    ),
    page(
        "`POST /_repos`, `DELETE /_repos`, `GET /_repos?nonce=N`",
        "Adds a repository to the running server for `git terrace open`, or takes one off for `git terrace remove`: loopback, no browser, secret from a 0600 file. The GET proves the secret by hashing it with the nonce",
        &["/_repos"],
    ),
    page(
        "`/_settings`, `/_settings/file`, `POST /_settings/check`",
        concat!(
            "The settings page: a form and a plain editor for `config.toml` and `actions.toml`, ",
            "read, checked and saved on local servers only, same-origin with the page token, ",
            "never a path from the request"
        ),
        &["/_settings", "/_settings/file", "/_settings/check"],
    ),
];
