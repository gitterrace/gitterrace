//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move.

use schemars::JsonSchema;
use serde::Serialize;

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
