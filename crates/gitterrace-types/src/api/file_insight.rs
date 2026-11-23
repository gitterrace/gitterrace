//! The answer of `GET /file_insight`: the card of one file or directory. Fields are in
//! alphabetical order, as in the rest of this module.

use schemars::JsonSchema;
use serde::Serialize;

/// What the path is at the revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileInsightKind {
    File,
    Dir,
    /// Not a text file or a directory of the tree: the card is empty, not an error.
    Missing,
}

/// When the path last changed and how old its lines are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FileInsightAge {
    /// Author time of the newest counted change, null when there is none.
    pub last_change: Option<i64>,
    /// Median age of the lines now in seconds, from blame. Null for a directory, a file
    /// that is not in the tree and a file blame cannot read.
    pub median_line_age_secs: Option<i64>,
}

/// How much the path changed in the period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FileInsightChurn {
    pub changes: usize,
    /// Ranked files: 0 when the path is not ranked.
    pub of: usize,
    /// Added plus removed lines in the period.
    pub lines_changed: u64,
    /// 1-based by hotspot score, for a directory its best file. 0 when not ranked.
    pub rank: usize,
    /// Changes per bucket over the period, oldest first, 12 buckets.
    pub series: Vec<u32>,
}

/// One person's share of the path's knowledge.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FileInsightOwner {
    pub name: String,
    /// 0 to 1.
    pub share: f64,
}

/// A file that changes in the same commits as the card's file.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FileInsightPartner {
    /// `together / min(changes of the two files)`, 0 to 1.
    pub degree: f64,
    pub path: String,
    pub together: usize,
}

/// A direct child of a directory card, for a tree badge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FileInsightChild {
    pub changes: usize,
    /// In the top tenth of files by hotspot score.
    pub hot: bool,
    pub path: String,
    /// 1-based by hotspot score, the best file under a directory. 0 when not ranked.
    pub rank: usize,
}

/// `GET /file_insight`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FileInsight {
    pub age: FileInsightAge,
    /// People with a counted change in the 90 days before the newest commit.
    pub authors_90d: usize,
    /// People who together hold half the knowledge, 0 when no counted change.
    pub bus_factor: usize,
    pub churn: FileInsightChurn,
    /// In the top tenth of files by hotspot score: the hotspots page's own rule.
    pub hot: bool,
    pub kind: FileInsightKind,
    pub link: Option<String>,
    /// Top 3 by share.
    pub owners: Vec<FileInsightOwner>,
    /// Top 3 by commits in common, files only.
    pub partners: Vec<FileInsightPartner>,
    pub path: String,
    pub permalink: Option<String>,
    /// Directories only: each direct child.
    pub paths: Vec<FileInsightChild>,
    pub rev: String,
}
