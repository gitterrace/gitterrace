//! Registry of the aggregates in `gitterrace_index`. To expose a new aggregate, add one
//! entry to [`INSIGHTS`]; the HTTP route `/insights/{name}` and the docs list pick it up.

use serde::Deserialize;
use serde_json::{Value, to_value};
use gitterrace_index::{CommitRecord, Granularity, Label};

use crate::error::ApiError;

/// Query parameters any insight may read. Unused ones are ignored.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Params {
    /// `day`, `week` or `month`.
    pub granularity: Option<String>,
    /// `kind` or `scope`.
    pub label: Option<String>,
    /// Unix seconds; only activity after this counts (hotspots).
    pub since: Option<i64>,
    pub limit: Option<usize>,
}

type Compute = fn(&[CommitRecord], &Params) -> Result<Value, ApiError>;

/// One named aggregate.
pub struct Insight {
    pub name: &'static str,
    pub summary: &'static str,
    compute: Compute,
}

/// Every aggregate exposed over HTTP.
pub const INSIGHTS: &[Insight] = &[
    Insight {
        name: "totals",
        summary: "Commit, author, file and line totals.",
        compute: |r, _| val(&gitterrace_index::totals(r)),
    },
    Insight {
        name: "activity",
        summary: "Commits and lines per day, week or month (granularity).",
        compute: |r, p| val(&gitterrace_index::activity(r, granularity(p)?)),
    },
    Insight {
        name: "heatmap",
        summary: "Commit counts by weekday and hour.",
        compute: |r, _| val(&gitterrace_index::heatmap(r)),
    },
    Insight {
        name: "authors",
        summary: "Per-author commits, lines and active days.",
        compute: |r, _| val(&gitterrace_index::authors(r)),
    },
    Insight {
        name: "file_churn",
        summary: "Per-file commits and lines changed, all time.",
        compute: |r, p| val(&limited(gitterrace_index::file_churn(r), p)),
    },
    Insight {
        name: "hotspots",
        summary: "Most changed files since a timestamp (since, limit).",
        compute: |r, p| {
            val(&gitterrace_index::hotspots(
                r,
                p.since.unwrap_or(0),
                p.limit.unwrap_or(20),
            ))
        },
    },
    Insight {
        name: "dir_churn",
        summary: "Per-directory commits and lines changed.",
        compute: |r, p| val(&limited(gitterrace_index::dir_churn(r), p)),
    },
    Insight {
        name: "label_breakdown",
        summary: "Conventional-commit type or scope counts (label).",
        compute: |r, p| val(&gitterrace_index::label_breakdown(r, label(p)?)),
    },
    Insight {
        name: "label_over_time",
        summary: "Type or scope counts per bucket (label, granularity).",
        compute: |r, p| {
            val(&gitterrace_index::label_over_time(
                r,
                label(p)?,
                granularity(p)?,
            ))
        },
    },
];

/// Runs the named insight.
pub fn compute(name: &str, records: &[CommitRecord], params: &Params) -> Result<Value, ApiError> {
    let insight = INSIGHTS
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown insight: {name}")))?;
    (insight.compute)(records, params)
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

fn granularity(p: &Params) -> Result<Granularity, ApiError> {
    match p.granularity.as_deref().unwrap_or("month") {
        "day" => Ok(Granularity::Day),
        "week" => Ok(Granularity::Week),
        "month" => Ok(Granularity::Month),
        other => Err(ApiError::BadRequest(format!(
            "granularity must be day, week or month, got {other}"
        ))),
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
