//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move. `permalink` and `link` sit at their sorted place
//! (no flatten, which would break the order); both are null for a repository without commits.

use schemars::JsonSchema;
use serde::Serialize;

use crate::api::Signature;
use crate::git::{ChangeKind, LineKind};

/// One commit of a log page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct LogCommit {
    pub author: Signature,
    pub committer: Signature,
    pub id: String,
    pub link: Option<String>,
    pub parents: Vec<String>,
    pub permalink: Option<String>,
    pub subject: String,
}

/// `GET /log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct LogResponse {
    pub commits: Vec<LogCommit>,
    pub has_more: bool,
    pub page: usize,
    pub path: Option<String>,
    pub per_page: usize,
    pub rev: String,
}

/// One line of a diff hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DiffLine {
    pub kind: LineKind,
    pub new_no: Option<u32>,
    pub old_no: Option<u32>,
    pub text: String,
}

/// A run of changed lines under one header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// One file a commit changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FileItem {
    pub additions: u32,
    pub binary: bool,
    pub deletions: u32,
    /// Absent unless the request asked for the diff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hunks: Option<Vec<DiffHunk>>,
    pub kind: ChangeKind,
    pub link: Option<String>,
    pub old_path: Option<String>,
    pub path: String,
    pub permalink: Option<String>,
}

/// `GET /compare`: the files that differ between two revisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CompareResponse {
    pub additions: u64,
    pub base: String,
    pub deletions: u64,
    pub files: Vec<FileItem>,
    /// The commit the head was compared from: `base`, or the merge base of `base` and `head`
    /// for a three-dot compare.
    pub from: String,
    pub head: String,
    pub path: Option<String>,
    pub three_dot: bool,
    /// The diff was cut at the line cap.
    pub truncated: bool,
}

/// `GET /worktree`: uncommitted changes against HEAD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct WorktreeResponse {
    pub additions: u64,
    pub deletions: u64,
    pub files: Vec<FileItem>,
    /// `all`, `staged` or `unstaged`.
    pub scope: String,
    /// The diff was cut at the line cap.
    pub truncated: bool,
}

/// `GET /commits/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CommitResponse {
    pub author: Signature,
    pub body: String,
    pub committer: Signature,
    pub files: Vec<FileItem>,
    pub id: String,
    pub link: Option<String>,
    pub parents: Vec<String>,
    pub permalink: Option<String>,
    pub subject: String,
    /// The diff was cut at the line cap.
    pub truncated: bool,
}
