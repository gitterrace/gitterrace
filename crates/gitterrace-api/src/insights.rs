//! Registry of the statistics the API serves, each one call on [`Insights`], the layer the web
//! pages read too. To expose a new one, add an entry to [`INSIGHTS`]; the HTTP route
//! `/insights/{name}` and the docs list pick it up.

use gitterrace_index::{
    Counted, Label, Metric, OptionParams, Period, Scope, ScopeParams, Selection,
};
use serde::Deserialize;
use serde_json::{Value, to_value};

use crate::error::ApiError;

/// Query parameters any insight may read.
///
/// Unused ones are ignored. The selection and option
/// parameters are the web pages' own (`type`, `scope`, `author`, `path`, `bots`, `generated`,
/// `ignore`, `ws`, `exclude`), so a page URL's query gives the same numbers here.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Params {
    /// `1w`, `1m`, `3m`, `1y` or `all`.
    pub period: Option<String>,
    /// `kind` or `scope`.
    pub label: Option<String>,
    /// `commits` (default), `added`, `deleted` or `net`: what `contributors` orders by.
    pub metric: Option<String>,
    #[serde(default, deserialize_with = "number")]
    pub limit: Option<usize>,
    #[serde(flatten)]
    pub select: Selection,
    #[serde(flatten)]
    pub options: OptionParams,
}

type Compute = fn(&Counted<'_>, &Params) -> Result<Value, ApiError>;

/// One named statistic.
pub struct Insight {
    pub name: &'static str,
    pub summary: &'static str,
    compute: Compute,
}

/// Every statistic exposed over HTTP.
pub const INSIGHTS: &[Insight] = &[
    Insight {
        name: "summary",
        summary: "Commits, authors, files and lines in the period, and in the period before it \
            (period).",
        compute: |c, _| val(&c.summary()),
    },
    Insight {
        name: "activity",
        summary: "Commits and lines per bucket, empty ones included, plus the weekday-by-hour \
            punch card. The period sets the unit (period).",
        compute: |c, _| val(&c.rhythm()),
    },
    Insight {
        name: "contributors",
        summary: "People with commits, lines, active days and a per-bucket series, bots flagged \
            (period).",
        compute: |c, q| {
            let m = q
                .metric
                .as_deref()
                .map_or_else(Default::default, Metric::parse);
            let mut r = c.contributors(m);
            r.contributors.truncate(q.limit.unwrap_or(usize::MAX));
            val(&r)
        },
    },
    Insight {
        name: "file_churn",
        summary: "Per-file commits and lines changed (period, limit).",
        compute: |c, q| val(&limited(c.per_file(), q)),
    },
    Insight {
        name: "dir_churn",
        summary: "Per-directory commits and lines changed (period, limit).",
        compute: |c, q| val(&limited(c.per_dir(), q)),
    },
    Insight {
        name: "label_breakdown",
        summary: "Conventional-commit type or scope counts (label, period).",
        compute: |c, q| val(&c.per_label(label(q)?).0),
    },
    Insight {
        name: "label_over_time",
        summary: "Type or scope counts per bucket, empty ones included (label, period).",
        compute: |c, q| val(&c.per_label(label(q)?).1),
    },
    Insight {
        name: "frequency",
        summary: "Lines added, removed and rewritten per bucket (period).",
        compute: |c, _| val(&c.frequency().1),
    },
];

/// Directory depth ownership groups by: two levels, as the web Ownership page shows it.
pub const OWNERSHIP_DEPTH: usize = 2;

/// Statistics that also read the head tree's files and line counts.
pub const HEAD_INSIGHTS: &[(&str, &str)] = &[
    (
        "hotspots",
        "Files of the head tree by changes times the square root of their size (period, limit).",
    ),
    (
        "ownership",
        "Knowledge share and bus factor per directory two levels deep, weighted by size now \
        (period).",
    ),
];

/// Every insight name with its summary, in documentation order.
pub fn list() -> impl Iterator<Item = (&'static str, &'static str)> {
    INSIGHTS
        .iter()
        .map(|i| (i.name, i.summary))
        .chain(HEAD_INSIGHTS.iter().copied())
}

/// The request's [`Scope`]: the period (default `all`), the selection and the options over
/// `defaults`, through the one constructor the web pages use too.
pub fn scope(params: &Params, defaults: gitterrace_index::Options) -> Result<Scope, ApiError> {
    let p = ScopeParams {
        period: params.period.clone(),
        select: params.select.clone(),
        options: params.options.clone(),
        ..ScopeParams::default()
    };
    Scope::from_params(&p, defaults, Period::All).map_err(|e| ApiError::BadRequest(e.to_string()))
}

/// Runs the named insight over the commits `c` counts. `head` is called only for the
/// head-tree insights.
pub fn compute(
    name: &str,
    c: &Counted<'_>,
    params: &Params,
    head: impl FnOnce() -> Vec<gitterrace_index::HeadFile>,
) -> Result<Value, ApiError> {
    let ins = c.insights();
    match name {
        "hotspots" => {
            let mut r = ins.hotspots_in(&head(), c);
            r.files.truncate(params.limit.unwrap_or(20));
            val(&r)
        }
        "ownership" => val(&ins.ownership_in(&head(), c, OWNERSHIP_DEPTH)),
        _ => {
            let insight = INSIGHTS
                .iter()
                .find(|i| i.name == name)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown insight: {name}")))?;
            (insight.compute)(c, params)
        }
    }
}

/// `GET /insights/{name}`. `data` is the insight's own shape, typed per insight later.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InsightResponse {
    pub commits_indexed: usize,
    pub data: Value,
    pub insight: String,
    pub rev: String,
}

/// Adds `permalink` and `link` from `links` to every row of the per-file insights.
pub(crate) fn with_links(
    name: &str,
    mut data: Value,
    links: impl Fn(&str) -> (Option<String>, Option<String>),
) -> Value {
    let rows = match name {
        "file_churn" => data.as_array_mut(),
        "hotspots" => data["files"].as_array_mut(),
        _ => None,
    };
    for row in rows.into_iter().flatten() {
        if let Some(p) = row["path"].as_str().map(str::to_owned) {
            let (permalink, link) = links(&p);
            row["permalink"] = Value::from(permalink);
            row["link"] = Value::from(link);
        }
    }
    data
}

fn val<T: serde::Serialize>(v: &T) -> Result<Value, ApiError> {
    to_value(v).map_err(|e| ApiError::Task(e.to_string()))
}

fn limited<T>(mut v: Vec<T>, p: &Params) -> Vec<T> {
    if let Some(n) = p.limit {
        v.truncate(n);
    }
    v
}

/// A number from a query string or JSON. `flatten` hands every query value over as a string.
fn number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<usize>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum N {
        N(usize),
        S(String),
    }
    match Option::<N>::deserialize(d)? {
        None => Ok(None),
        Some(N::N(n)) => Ok(Some(n)),
        Some(N::S(s)) if s.is_empty() => Ok(None),
        Some(N::S(s)) => s.parse().map(Some).map_err(serde::de::Error::custom),
    }
}

fn label(p: &Params) -> Result<Label, ApiError> {
    match p.label.as_deref().unwrap_or("kind") {
        "kind" => Ok(Label::Kind),
        "scope" => Ok(Label::Scope),
        other => Err(ApiError::BadRequest(format!(
            "label must be kind or scope, got {other}"
        ))),
    }
}
