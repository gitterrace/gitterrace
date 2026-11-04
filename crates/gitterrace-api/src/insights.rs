//! Registry of the statistics the API serves, each one call on [`Insights`], the layer the web
//! pages read too. To expose a new one, add an entry to [`INSIGHTS`]; the HTTP route
//! `/insights/{name}` and the docs list pick it up.

use gitterrace_index::{Insights, Label, Metric, OptionParams, Period, Selection};
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

type Compute = fn(&Insights<'_>, Period, &Params) -> Result<Value, ApiError>;

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
        summary: "Commits, authors, files and lines in the period, and in the period before it (period).",
        compute: |i, p, _| val(&i.period_summary(p)),
    },
    Insight {
        name: "activity",
        summary: "Commits and lines per bucket, empty ones included, plus the weekday-by-hour punch card. The period sets the unit (period).",
        compute: |i, p, _| val(&i.rhythm(p)),
    },
    Insight {
        name: "contributors",
        summary: "People with commits, lines, active days and a per-bucket series, bots flagged (period).",
        compute: |i, p, q| {
            let m = q
                .metric
                .as_deref()
                .map_or_else(Default::default, Metric::parse);
            let mut r = i.contributors_by(p, m);
            r.contributors.truncate(q.limit.unwrap_or(usize::MAX));
            val(&r)
        },
    },
    Insight {
        name: "file_churn",
        summary: "Per-file commits and lines changed (period, limit).",
        compute: |i, p, q| val(&limited(i.file_churn(p), q)),
    },
    Insight {
        name: "dir_churn",
        summary: "Per-directory commits and lines changed (period, limit).",
        compute: |i, p, q| val(&limited(i.dir_churn(p), q)),
    },
    Insight {
        name: "label_breakdown",
        summary: "Conventional-commit type or scope counts (label, period).",
        compute: |i, p, q| val(&i.label_breakdown(label(q)?, p)),
    },
    Insight {
        name: "label_over_time",
        summary: "Type or scope counts per bucket, empty ones included (label, period).",
        compute: |i, p, q| val(&i.label_over_time(label(q)?, p)),
    },
    Insight {
        name: "frequency",
        summary: "Lines added, removed and rewritten per bucket (period).",
        compute: |i, p, _| val(&i.frequency(p).1),
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
        "Knowledge share and bus factor per directory two levels deep, weighted by size now (period).",
    ),
];

/// Every insight name with its summary, in documentation order.
pub fn list() -> impl Iterator<Item = (&'static str, &'static str)> {
    INSIGHTS
        .iter()
        .map(|i| (i.name, i.summary))
        .chain(HEAD_INSIGHTS.iter().copied())
}

/// Runs the named insight over `ins`. `head` is called only for the head-tree insights.
pub fn compute(
    name: &str,
    ins: &Insights<'_>,
    params: &Params,
    head: impl FnOnce() -> Vec<gitterrace_index::HeadFile>,
) -> Result<Value, ApiError> {
    let p = period(params)?;
    match name {
        "hotspots" => {
            let mut r = ins.hotspots(&head(), p);
            r.files.truncate(params.limit.unwrap_or(20));
            val(&r)
        }
        "ownership" => val(&ins.ownership(&head(), p, OWNERSHIP_DEPTH)),
        _ => {
            let insight = INSIGHTS
                .iter()
                .find(|i| i.name == name)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown insight: {name}")))?;
            (insight.compute)(ins, p, params)
        }
    }
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

fn period(p: &Params) -> Result<Period, ApiError> {
    p.period
        .as_deref()
        .filter(|s| !s.is_empty())
        .map_or(Ok(Period::All), |s| {
            Period::parse(s).ok_or_else(|| {
                ApiError::BadRequest(format!("period must be 1w, 1m, 3m, 1y or all, got {s}"))
            })
        })
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
