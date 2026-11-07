//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move.

use schemars::JsonSchema;
use serde::Serialize;

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
