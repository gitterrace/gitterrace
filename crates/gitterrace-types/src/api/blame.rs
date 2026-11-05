//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move.

use schemars::JsonSchema;
use serde::Serialize;

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
