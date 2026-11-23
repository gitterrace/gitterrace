//! The shapes of the JSON API outside the insights: one response struct per endpoint, shared by
//! the HTTP routes, the MCP tools and the generated `OpenAPI` document.
//!
//! Every struct declares its fields in alphabetical order: the JSON these replace was a
//! `serde_json` map, which sorts keys, and the bytes must not move. Never reorder fields.

use std::borrow::Cow;

use schemars::JsonSchema;
use serde::Serialize;

use crate::git::{RefKind, TreeEntryKind};

mod commit;
mod insights;
mod review;

#[cfg(test)]
mod tests;

pub use commit::{
    CommitResponse, CompareResponse, DiffHunk, DiffLine, FileItem, LogCommit, LogResponse,
    WorktreeResponse,
};
pub use insights::{
    InsightContributor, InsightContributors, InsightData, InsightDirChurn, InsightDirOwnership,
    InsightFileChurn, InsightFrequency, InsightHotspot, InsightHotspots, InsightLabelBucket,
    InsightLabelCount, InsightLanguage, InsightLanguages, InsightOwner, InsightOwnership,
    InsightPeriod, InsightPeriodStats, InsightPeriodSummary, InsightResponse, InsightRhythm,
    InsightSeriesPoint, InsightUnit,
};
pub use review::{
    CheckpointItem, CheckpointsResponse, Finding, FindingAnchor, FindingLines, FindingsResponse,
    NewFinding, ViewedItem, ViewedResponse,
};

/// Who did something and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Signature {
    pub email: String,
    pub name: String,
    /// Offset from UTC in seconds.
    pub offset: i32,
    /// Seconds since the Unix epoch.
    pub time: i64,
}

/// The commit that last changed one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BlameItem {
    pub author: String,
    pub commit: String,
    pub line_no: u32,
    pub text: String,
    pub time: i64,
}

/// `GET /blame`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BlameResponse {
    pub commit: String,
    pub lines: Vec<BlameItem>,
    pub link: Option<String>,
    pub path: String,
    pub permalink: Option<String>,
    pub rev: String,
    pub truncated: bool,
}

/// `GET /blob`. `text` and `total_lines` are null for a binary file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BlobResponse {
    pub binary: bool,
    pub commit: String,
    pub link: Option<String>,
    pub path: String,
    pub permalink: Option<String>,
    pub rev: String,
    pub size: u64,
    pub text: Option<String>,
    pub total_lines: Option<usize>,
    pub truncated: bool,
}

/// One person with identities merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ContributorItem {
    pub active_days: usize,
    pub additions: u64,
    pub bot: bool,
    pub commits: usize,
    pub deletions: u64,
    pub emails: Vec<String>,
    pub first_commit: i64,
    /// The first commit ever falls inside the counted window. Never for the whole history.
    pub first_time: bool,
    pub last_commit: i64,
    pub name: String,
    /// Commits per bucket.
    pub series: Vec<u32>,
}

/// `GET /contributors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ContributorsResponse {
    pub authors: Vec<ContributorItem>,
    /// The fewest people who made half the commits.
    pub bus_factor: usize,
    pub rev: String,
}

/// One branch, tag or remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RefItem {
    pub kind: RefKind,
    pub name: String,
    pub target: String,
    pub time: i64,
}

/// `GET /refs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RefsResponse {
    pub refs: Vec<RefItem>,
}

/// Commits, authors, files and lines over the whole history. Every field but `every_commits`
/// counts the counted commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RepoTotals {
    pub additions: u64,
    pub authors: usize,
    /// The counted commits: no merges, bots as the options say.
    pub commits: usize,
    pub deletions: u64,
    /// Every reachable commit, merges and bots included: the overview's "N commits".
    pub every_commits: u64,
    pub files: usize,
}

/// `GET /repo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RepoResponse {
    pub head: Option<String>,
    pub head_branch: Option<String>,
    pub index_version: Option<String>,
    pub name: String,
    pub totals: RepoTotals,
}

/// One entry of a directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TreeItem {
    pub kind: TreeEntryKind,
    pub name: String,
    pub size: Option<u64>,
}

/// `GET /tree`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TreeResponse {
    pub commit: String,
    pub entries: Vec<TreeItem>,
    pub path: String,
    pub rev: String,
}

/// One query parameter of an endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ParamDoc {
    pub description: Cow<'static, str>,
    pub name: Cow<'static, str>,
    pub required: bool,
    pub ty: Cow<'static, str>,
}

/// One endpoint, as `GET /` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct EndpointDoc {
    pub params: Vec<ParamDoc>,
    pub path: String,
    /// The top-level fields of the answer.
    pub returns: String,
    pub summary: String,
}

/// One available insight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct InsightRow {
    pub name: String,
    pub summary: String,
}

/// `GET /insights`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct InsightList {
    pub insights: Vec<InsightRow>,
}

/// `GET /`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct IndexResponse {
    pub endpoints: Vec<EndpointDoc>,
    pub insights: Vec<InsightRow>,
    pub version: u32,
}

/// Which commits a count covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Population {
    /// The commits every insight counts: no merges, bots as the options say, the selection
    /// and the period applied.
    Counted,
    /// Every reachable commit, merges and bots included: the log and the overview.
    Every,
}

impl Population {
    /// The word a page writes in its `data-population` attribute.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Counted => "counted",
            Self::Every => "every",
        }
    }
}

/// A count and its population, as the API carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FigureItem {
    pub population: Population,
    pub value: u64,
}
