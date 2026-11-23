//! The one registry of HTTP endpoints. [`crate::router`] builds its routes from it, `GET /`
//! returns it and the generated API reference renders it, so the three cannot drift.

use std::borrow::Cow;

use axum::routing::{MethodRouter, get};
use gitterrace_types::api::{
    BlameResponse, BlobResponse, CheckpointsResponse, CommitResponse, CompareResponse,
    ContributorsResponse, FileInsight, FindingsResponse, IndexResponse, InsightList,
    InsightResponse, LogResponse, ParamDoc as Param, RefsResponse, RepoResponse, ReviewPlan,
    TreeResponse, ViewedResponse, WorktreeResponse,
};
use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings};
use serde::Serialize;

use crate::core::ApiState;
use crate::http;

/// One `GET` endpoint under `/api/v1`.
#[derive(Serialize)]
pub struct Endpoint {
    pub path: &'static str,
    pub summary: &'static str,
    pub params: &'static [Param],
    /// The response type. It names its definition in the generator and returns a reference to it.
    #[serde(skip)]
    pub schema: fn(&mut SchemaGenerator) -> Schema,
    #[serde(skip)]
    pub(crate) route: fn() -> MethodRouter<ApiState>,
}

impl Endpoint {
    /// The top-level fields of the answer, from its schema: the one source of what an endpoint
    /// returns.
    pub fn fields(&self, generator: &mut SchemaGenerator) -> Vec<String> {
        let schema = (self.schema)(generator);
        let object = schema.get("$ref").and_then(|r| r.as_str()).map_or_else(
            || schema.clone().to_value(),
            |r| {
                r.rsplit('/')
                    .next()
                    .and_then(|name| generator.definitions().get(name))
                    .cloned()
                    .unwrap_or_default()
            },
        );
        object
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// [`Endpoint::fields`] as a comma separated line.
    pub fn returns(&self, generator: &mut SchemaGenerator) -> String {
        self.fields(generator).join(", ")
    }
}

/// A generator whose references point into `components/schemas`, where the `OpenAPI` document
/// keeps the definitions.
pub fn generator() -> SchemaGenerator {
    SchemaGenerator::new(
        SchemaSettings::draft2020_12().with(|s| s.definitions_path = "/components/schemas".into()),
    )
}

fn of<T: JsonSchema>(g: &mut SchemaGenerator) -> Schema {
    g.subschema_for::<T>()
}

const fn p(name: &'static str, ty: &'static str, description: &'static str) -> Param {
    Param {
        name: Cow::Borrowed(name),
        ty: Cow::Borrowed(ty),
        required: false,
        description: Cow::Borrowed(description),
    }
}

const fn req(name: &'static str, ty: &'static str, description: &'static str) -> Param {
    Param {
        name: Cow::Borrowed(name),
        ty: Cow::Borrowed(ty),
        required: true,
        description: Cow::Borrowed(description),
    }
}

const REV: Param = p("rev", "string", "Revision to read, default `HEAD`.");
// The period, the selection and the options: the web pages' own parameters, shared by every
// endpoint that counts commits.
const Q: Param = p(
    "q",
    "string",
    "The commit filter, the web filter field's text, see Filter. The one way to select \
    commits.",
);
const PERIOD: Param = p(
    "period",
    "string",
    "`1w`, `1m`, `3m`, `1y` or `all` (default): the window, ending at the newest \
    commit. Sets the bucket unit.",
);
const BOTS: Param = p("bots", "string", "`1` counts bot accounts, `0` leaves them out.");
const MERGES: Param =
    p("merges", "string", "`1` counts merge commits as commits, never their lines or files.");
const GENERATED: Param = p("generated", "string", "`1` counts lock files and minified output.");
const IGNORE: Param = p("ignore", "string", "`0` counts paths that `.gitignore` rules out.");
const WS: Param = p("ws", "string", "`1` counts whitespace-only changes.");
const EXCLUDE: Param = p("exclude", "string", "Comma-separated globs left out.");
const PATH: Param =
    p("path", "string", "Directory to list, relative to the repository root, default the root.");

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
        schema: of::<IndexResponse>,
        route: || get(http::index),
    },
    Endpoint {
        path: "/repo",
        summary: "Repository identity and totals.",
        params: &[],
        schema: of::<RepoResponse>,
        route: || get(http::repo),
    },
    Endpoint {
        path: "/refs",
        summary: "Branches, tags and remotes.",
        params: &[],
        schema: of::<RefsResponse>,
        route: || get(http::refs),
    },
    Endpoint {
        path: "/log",
        summary: "Paged history.",
        params: &[
            REV,
            p("path", "string", "Only commits touching this path."),
            p("page", "integer", "1-based page, default `1`."),
            p("per_page", "integer", "Page size, default `50`, at most `api.log_page_commits`."),
        ],
        schema: of::<LogResponse>,
        route: || get(http::log),
    },
    Endpoint {
        path: "/commits/{id}",
        summary: "One commit with its diff.",
        params: &[
            p("diff", "boolean", "Include hunks, default `true`."),
            p("max_lines", "integer", "Most diff lines, at most `api.diff_lines`."),
        ],
        schema: of::<CommitResponse>,
        route: || get(http::commit),
    },
    Endpoint {
        path: "/compare",
        summary: "Files that differ between two revisions, with diffs.",
        params: &[
            req("base", "string", "Revision to compare from."),
            p("head", "string", "Revision to compare to, default `HEAD`."),
            p("path", "string", "Only files at or under this path."),
            p(
                "three_dot",
                "boolean",
                "`true` compares from the merge base of `base` and `head`, like `base...head`.",
            ),
            p("max_lines", "integer", "Most diff lines, at most `api.diff_lines`."),
        ],
        schema: of::<CompareResponse>,
        route: || get(http::compare),
    },
    Endpoint {
        path: "/worktree",
        summary: "Uncommitted changes against HEAD. Refused on a public server.",
        params: &[
            p("scope", "string", "`all` (default), `staged` or `unstaged`."),
            p("max_lines", "integer", "Most diff lines, at most `api.diff_lines`."),
            p(
                "since",
                "string",
                "A checkpoint id from `/checkpoints`: list only the files that differ from it. \
                Needs scope `all`. An unknown id is 404 `checkpoint_not_found`.",
            ),
        ],
        schema: of::<WorktreeResponse>,
        route: || get(http::worktree),
    },
    Endpoint {
        path: "/checkpoints",
        summary: "Checkpoints of the uncommitted work, newest first. Refused on a public server.",
        params: &[],
        schema: of::<CheckpointsResponse>,
        route: || get(http::checkpoints),
    },
    Endpoint {
        path: "/viewed",
        summary: "Files marked viewed, with the patch hash each mark holds for. Refused on a \
            public server.",
        params: &[],
        schema: of::<ViewedResponse>,
        route: || get(http::viewed),
    },
    Endpoint {
        path: "/findings",
        summary: "The review basket: findings with their notes, oldest first, and where the \
            code of each is now. Refused on a public server.",
        params: &[],
        schema: of::<FindingsResponse>,
        route: || get(http::findings),
    },
    Endpoint {
        path: "/tree",
        summary: "One directory level.",
        params: &[REV, PATH],
        schema: of::<TreeResponse>,
        route: || get(http::tree),
    },
    Endpoint {
        path: "/blob",
        summary: "File text at a revision.",
        params: &[
            REV,
            req("path", "string", "File to read."),
            p("start_line", "integer", "First line of the window, 1-based, default `1`."),
            p("limit", "integer", "Lines in the window, at most `api.blob_lines`."),
        ],
        schema: of::<BlobResponse>,
        route: || get(http::blob),
    },
    Endpoint {
        path: "/blame",
        summary: "Per-line authorship.",
        params: &[REV, req("path", "string", "File to blame.")],
        schema: of::<BlameResponse>,
        route: || get(http::blame),
    },
    Endpoint {
        path: "/contributors",
        summary: "Authors and bus factor, counted as `/insights/contributors` counts them.",
        params: &[REV, Q, PERIOD, BOTS, MERGES, GENERATED, IGNORE, WS, EXCLUDE],
        schema: of::<ContributorsResponse>,
        route: || get(http::contributors),
    },
    Endpoint {
        path: "/file_insight",
        summary: "The card of one file or directory: churn rank and sparkline, owners, partners, \
            age.",
        params: &[
            REV,
            Q,
            PERIOD,
            req("path", "string", "File or directory, relative to the repository root."),
            BOTS,
            MERGES,
            GENERATED,
            IGNORE,
            WS,
            EXCLUDE,
        ],
        schema: of::<FileInsight>,
        route: || get(http::file_insight),
    },
    Endpoint {
        path: "/review_plan",
        summary: "The changed files in the order to review them, each with a risk and its \
            named terms.",
        params: &[
            p("source", "string", "`worktree` (default, refused on a public server) or `compare`."),
            p("base", "string", "`compare`: revision to compare from. Required there."),
            p("head", "string", "`compare`: revision to compare to, default `HEAD`."),
            p(
                "three_dot",
                "boolean",
                "`compare`: `true` compares from the merge base, like `/compare`.",
            ),
            p("scope", "string", "`worktree`: `all` (default), `staged` or `unstaged`."),
            p(
                "since",
                "string",
                "`worktree`: a checkpoint id from `/checkpoints`, only what changed since it. \
                Needs scope `all`.",
            ),
            Q,
            PERIOD,
            p("limit", "integer", "Most files listed, default and at most `review.plan_files`."),
            BOTS,
            MERGES,
            GENERATED,
            IGNORE,
            WS,
            EXCLUDE,
        ],
        schema: of::<ReviewPlan>,
        route: || get(http::review_plan),
    },
    Endpoint {
        path: "/insights",
        summary: "Available insights.",
        params: &[],
        schema: of::<InsightList>,
        route: || get(http::insight_list),
    },
    Endpoint {
        path: "/insights/{name}",
        summary: "One insight.",
        params: &[
            REV,
            Q,
            PERIOD,
            p("label", "string", "`kind` (default) or `scope`."),
            p(
                "metric",
                "string",
                "`contributors` only: `commits` (default), `added`, `deleted` or `net`. Sets the \
                order and the per-bucket series.",
            ),
            p("limit", "integer", "Most rows."),
            p(
                "path",
                "string",
                "`coupling`: the focus file, else the whole tree. `knowledge_risk`: the directory \
                to drill, else the root.",
            ),
            p(
                "author",
                "string",
                "`knowledge_risk` only: the person to report, as `ownership` names them.",
            ),
            p("min_support", "integer", "`coupling` only: shared commits a pair needs, default 3."),
            p("cross_dir", "string", "`coupling` only: `1` lists pairs across directories too."),
            BOTS,
            MERGES,
            GENERATED,
            IGNORE,
            WS,
            EXCLUDE,
        ],
        schema: of::<InsightResponse>,
        route: || get(http::insight),
    },
];
