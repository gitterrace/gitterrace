//! The one registry of HTTP endpoints. [`crate::router`] builds its routes from it, `GET /`
//! returns it and the generated API reference renders it, so the three cannot drift.

use axum::routing::{MethodRouter, get};
use serde::Serialize;

use crate::core::ApiState;
use crate::http;

/// One query parameter.
#[derive(Debug, Serialize)]
pub struct Param {
    pub name: &'static str,
    pub ty: &'static str,
    pub required: bool,
    pub description: &'static str,
}

/// One `GET` endpoint under `/api/v1`.
#[derive(Serialize)]
pub struct Endpoint {
    pub path: &'static str,
    pub summary: &'static str,
    pub params: &'static [Param],
    pub returns: &'static str,
    #[serde(skip)]
    pub(crate) route: fn() -> MethodRouter<ApiState>,
}

const fn p(name: &'static str, ty: &'static str, description: &'static str) -> Param {
    Param {
        name,
        ty,
        required: false,
        description,
    }
}

const fn req(name: &'static str, ty: &'static str, description: &'static str) -> Param {
    Param {
        name,
        ty,
        required: true,
        description,
    }
}

const REV: Param = p("rev", "string", "Revision to read, default `HEAD`.");
const PATH: Param = p("path", "string", "Repository-relative path.");

/// [`ENDPOINTS`] behind a function call: `index` is itself a route in the registry, and naming the
/// const inside its own body would be a type cycle.
pub(crate) const fn all() -> &'static [Endpoint] {
    ENDPOINTS
}

/// Every endpoint. Add one here and it is routed, listed and documented.
pub const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        path: "/",
        summary: "Endpoint and insight list.",
        params: &[],
        returns: "version, endpoints, insights",
        route: || get(http::index),
    },
    Endpoint {
        path: "/repo",
        summary: "Repository identity and totals.",
        params: &[],
        returns: "name, head, head_branch, totals",
        route: || get(http::repo),
    },
    Endpoint {
        path: "/refs",
        summary: "Branches, tags and remotes.",
        params: &[],
        returns: "refs",
        route: || get(http::refs),
    },
    Endpoint {
        path: "/log",
        summary: "Paged history.",
        params: &[
            REV,
            p("path", "string", "Only commits touching this path."),
            p("page", "integer", "1-based page, default 1."),
            p("per_page", "integer", "Page size, default 50, capped."),
        ],
        returns: "commits, has_more",
        route: || get(http::log),
    },
    Endpoint {
        path: "/commits/{id}",
        summary: "One commit with its diff.",
        params: &[
            p("diff", "boolean", "Include hunks, default true."),
            p("max_lines", "integer", "Cap on diff lines."),
        ],
        returns: "message, files, hunks, truncated",
        route: || get(http::commit),
    },
    Endpoint {
        path: "/tree",
        summary: "One directory level.",
        params: &[REV, PATH],
        returns: "entries",
        route: || get(http::tree),
    },
    Endpoint {
        path: "/blob",
        summary: "File text at a revision.",
        params: &[
            REV,
            req("path", "string", "File to read."),
            p("start_line", "integer", "First line of the window."),
            p("limit", "integer", "Lines in the window, capped."),
        ],
        returns: "text, total_lines, truncated, binary",
        route: || get(http::blob),
    },
    Endpoint {
        path: "/blame",
        summary: "Per-line authorship.",
        params: &[REV, req("path", "string", "File to blame.")],
        returns: "lines (commit, author, time), truncated",
        route: || get(http::blame),
    },
    Endpoint {
        path: "/contributors",
        summary: "Authors and bus factor.",
        params: &[REV],
        returns: "authors, bus_factor",
        route: || get(http::contributors),
    },
    Endpoint {
        path: "/insights",
        summary: "Available insights.",
        params: &[],
        returns: "insights",
        route: || get(http::insight_list),
    },
    Endpoint {
        path: "/insights/{name}",
        summary: "One insight.",
        params: &[
            REV,
            p(
                "period",
                "string",
                "`1w`, `1m`, `3m`, `1y` or `all` (default): the window, ending at the newest commit. Sets the bucket unit.",
            ),
            p("label", "string", "`kind` or `scope`."),
            p("limit", "integer", "Cap on rows."),
            p(
                "type",
                "string",
                "Only commits of this conventional type, `other` for none.",
            ),
            p(
                "scope",
                "string",
                "Only commits of this scope, `none` for none.",
            ),
            p("author", "string", "Only commits by this e-mail or name."),
            p("path", "string", "Only files under this directory."),
            p(
                "bots",
                "string",
                "`1` counts bot accounts, `0` leaves them out.",
            ),
            p(
                "generated",
                "string",
                "`1` counts lock files and minified output.",
            ),
            p(
                "ignore",
                "string",
                "`0` counts paths that `.gitignore` rules out.",
            ),
            p("ws", "string", "`1` counts whitespace-only changes."),
            p("exclude", "string", "Comma separated globs left out."),
        ],
        returns: "{insight, data}",
        route: || get(http::insight),
    },
];
