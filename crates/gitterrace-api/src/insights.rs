//! Registry of the statistics the API serves, each one call on [`Insights`], the layer the web
//! pages read too. To expose a new one, add an entry to [`INSIGHTS`]: the HTTP route
//! `/insights/{name}`, the MCP `insight` tool, the `insights` command and the docs pick it up.

use std::sync::Arc;

use gitterrace_index::{
    Counted, HeadFile, Label, Metric, OptionParams, Period, Scope, ScopeParams, Selection,
};
use gitterrace_types::api::{InsightData, InsightList, InsightRow};
use serde::Deserialize;
use serde_json::{Value, to_value};

use crate::error::ApiError;

/// Query parameters any insight may read.
///
/// Unused ones are ignored. `q` is the commit filter, the web pages' own text. The selection
/// and option parameters are the web pages' too (`type`, `scope`, `author`, `path`, `bots`,
/// `generated`, `ignore`, `ws`, `exclude`), read as terms of `q`, so a page URL's query gives
/// the same numbers here.
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
    /// `coupling` only: the focus file.
    pub path: Option<String>,
    /// `coupling` only: shared commits a pair needs, default 3.
    #[serde(default, deserialize_with = "number")]
    pub min_support: Option<usize>,
    /// `coupling` only: `1` lists pairs across directories too, `0` (default) only siblings.
    pub cross_dir: Option<String>,
    #[serde(flatten)]
    pub select: Selection,
    #[serde(flatten)]
    pub options: OptionParams,
}

/// The head tree's files, read on first use: only the head-tree insights ask for them.
type Head<'a> = &'a dyn Fn() -> Arc<Vec<HeadFile>>;

impl Params {
    /// The parameters the MCP tool and the command line take, as the query string gives them.
    pub fn new(
        period: Option<String>,
        q: Option<String>,
        label: Option<String>,
        metric: Option<String>,
        limit: Option<usize>,
    ) -> Self {
        Self { period, label, metric, limit, select: Selection { q }, ..Self::default() }
    }
}

impl Params {
    /// These parameters with the ones only `coupling` reads set.
    #[must_use]
    pub fn coupling(
        mut self,
        path: Option<String>,
        min_support: Option<usize>,
        cross_dir: Option<String>,
    ) -> Self {
        (self.path, self.min_support, self.cross_dir) = (path, min_support, cross_dir);
        self
    }
}

type Compute = fn(&Counted<'_>, &Params, Head<'_>) -> Result<Value, ApiError>;

/// One named statistic.
pub struct Insight {
    pub name: &'static str,
    pub summary: &'static str,
    compute: Compute,
    /// Reads the computed value back as the typed `data` the schema is made from.
    typed: fn(Value) -> Result<InsightData, ApiError>,
}

/// Every statistic, served over HTTP (`/insights/{name}`), MCP (the `insight` tool) and the
/// command line (`insights <name>`) from this one list, in documentation order.
pub const INSIGHTS: &[Insight] = &[
    Insight {
        name: "summary",
        summary: "Commits, authors, files and lines in the period, and in the period before it.",
        compute: |c, _, _| val(&c.summary()),
        typed: |v| Ok(InsightData::Summary(read(v)?)),
    },
    Insight {
        name: "activity",
        summary: "Commits and lines per bucket, empty ones included, plus the weekday-by-hour \
            punch card. The period sets the unit.",
        compute: |c, _, _| val(&c.rhythm()),
        typed: |v| Ok(InsightData::Activity(Box::new(read(v)?))),
    },
    Insight {
        name: "contributors",
        summary: "People with commits, lines, active days and a per-bucket series, bots flagged. \
            Takes `metric` and `limit`.",
        compute: |c, q, _| {
            let m = q.metric.as_deref().map_or_else(Default::default, Metric::parse);
            let mut r = c.contributors(m);
            r.contributors.truncate(q.limit.unwrap_or(usize::MAX));
            val(&r)
        },
        typed: |v| Ok(InsightData::Contributors(read(v)?)),
    },
    Insight {
        name: "file_churn",
        summary: "Per-file commits and lines changed. Takes `limit`.",
        compute: |c, q, _| val(&limited(c.per_file(), q)),
        typed: |v| Ok(InsightData::FileChurn(read(v)?)),
    },
    Insight {
        name: "dir_churn",
        summary: "Per-directory commits and lines changed. Takes `limit`.",
        compute: |c, q, _| val(&limited(c.per_dir(), q)),
        typed: |v| Ok(InsightData::DirChurn(read(v)?)),
    },
    Insight {
        name: "label_breakdown",
        summary: "Conventional-commit type or scope counts. Takes `label`.",
        compute: |c, q, _| val(&c.per_label(label(q)?).0),
        typed: |v| Ok(InsightData::LabelBreakdown(read(v)?)),
    },
    Insight {
        name: "label_over_time",
        summary: "Type or scope counts per bucket, empty ones included. Takes `label`.",
        compute: |c, q, _| val(&c.per_label(label(q)?).1),
        typed: |v| Ok(InsightData::LabelOverTime(read(v)?)),
    },
    Insight {
        name: "frequency",
        summary: "Lines added, removed and rewritten per bucket.",
        compute: |c, _, _| val(&c.frequency().1),
        typed: |v| Ok(InsightData::Frequency(read(v)?)),
    },
    Insight {
        name: "hotspots",
        summary: "Files of the head tree by changes times the square root of their size. Takes \
            `limit`.",
        compute: |c, q, head| {
            let mut r = c.insights().hotspots_in(&head(), c);
            r.files.truncate(q.limit.unwrap_or(20));
            val(&r)
        },
        typed: |v| Ok(InsightData::Hotspots(read(v)?)),
    },
    Insight {
        name: "coupling",
        summary: "Files that change in the same commits, for one file or the whole tree. \
            Takes `path`, `min_support`, `cross_dir` and `limit`.",
        compute: |c, q, head| {
            let query = gitterrace_index::CouplingQuery {
                path: q.path.as_deref().map(|p| p.trim_matches('/')).filter(|p| !p.is_empty()),
                min_support: q.min_support.unwrap_or(COUPLING_MIN_SUPPORT).max(1),
                cross_dir: q.cross_dir.as_deref() == Some("1"),
                limit: q.limit.unwrap_or(COUPLING_LIMIT).min(COUPLING_MAX_LIMIT),
            };
            val(&c.insights().coupling_report_in(&head(), c, &query))
        },
        typed: |v| Ok(InsightData::Coupling(read(v)?)),
    },
    Insight {
        name: "languages",
        summary: "Lines and files per language in the head tree, and their total: the project's \
            size, as the web Languages page counts it.",
        compute: |c, _, head| val(&c.insights().languages_in(&head(), c)),
        typed: |v| Ok(InsightData::Languages(read(v)?)),
    },
    Insight {
        name: "ownership",
        summary: "Knowledge share and bus factor per directory two levels deep, weighted by size \
            now.",
        compute: |c, _, head| val(&c.insights().ownership_in(&head(), c, OWNERSHIP_DEPTH)),
        typed: |v| Ok(InsightData::Ownership(read(v)?)),
    },
];

/// Shared commits a coupled pair needs when the request names none.
/// Not configurable: the bar the patterns section has always used.
const COUPLING_MIN_SUPPORT: usize = 3;
/// Pairs `coupling` lists when the request names no limit.
/// Not configurable: a page of the strongest pairs.
const COUPLING_LIMIT: usize = 50;
/// Most pairs `coupling` lists, whatever `limit` asks.
/// Not configurable: it bounds the answer's size.
const COUPLING_MAX_LIMIT: usize = 500;

/// Refuses the parameters only `coupling` reads on any other insight: an unread key would
/// answer unfiltered numbers to a caller who thinks they are filtered.
pub(crate) fn check_params(name: &str, p: &Params) -> Result<(), ApiError> {
    if name == "coupling" {
        return Ok(());
    }
    let named = [
        ("path", p.path.is_some()),
        ("min_support", p.min_support.is_some()),
        ("cross_dir", p.cross_dir.is_some()),
    ];
    match named.iter().find(|(_, set)| *set) {
        Some((k, _)) => Err(ApiError::BadRequest(format!(
            "unknown parameter `{k}` for insight {name}: only coupling reads it. Select \
             commits with `q`"
        ))),
        None => Ok(()),
    }
}

/// Directory depth ownership groups by: two levels, as the web Ownership page shows it.
pub const OWNERSHIP_DEPTH: usize = 2;

/// Every insight name with its summary, in documentation order.
pub fn list() -> impl Iterator<Item = (&'static str, &'static str)> {
    INSIGHTS.iter().map(|i| (i.name, i.summary))
}

/// The `/insights` answer: every insight with its summary. HTTP, MCP and the command line list
/// from here.
pub fn index() -> InsightList {
    InsightList {
        insights: list()
            .map(|(name, summary)| InsightRow {
                name: name.to_owned(),
                summary: summary.to_owned(),
            })
            .collect(),
    }
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

/// The commits `scope` counts in `ins`, anchored on the whole source `all`: the one place an
/// API request reads the clock.
pub fn counted<'a>(
    ins: &'a gitterrace_index::Insights<'a>,
    scope: &'a Scope,
    all: &dyn gitterrace_index::feed::InsightsSource,
) -> Counted<'a> {
    ins.counted(
        scope,
        gitterrace_index::CountContext::new(all, &gitterrace_index::feed::SystemClock),
    )
}

/// Runs the named insight over the commits `c` counts. `head` is called only for the
/// head-tree insights.
pub fn compute(
    name: &str,
    c: &Counted<'_>,
    params: &Params,
    head: Head<'_>,
) -> Result<Value, ApiError> {
    check_params(name, params)?;
    (find(name)?.compute)(c, params, head)
}

/// The typed `data` of insight `name`, read back from the value `compute` and [`with_links`]
/// built, so the answer's shape is the one type the schema is made from.
pub(crate) fn typed(name: &str, data: Value) -> Result<InsightData, ApiError> {
    (find(name)?.typed)(data)
}

fn find(name: &str) -> Result<&'static Insight, ApiError> {
    INSIGHTS
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown insight: {name}")))
}

fn read<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError::Task(e.to_string()))
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
        other => Err(ApiError::BadRequest(format!("label must be kind or scope, got {other}"))),
    }
}
