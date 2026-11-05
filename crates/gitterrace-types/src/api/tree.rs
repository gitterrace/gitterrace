//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move.

use schemars::JsonSchema;
use serde::Serialize;

use crate::git::TreeEntryKind;

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
