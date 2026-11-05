//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move.

use schemars::JsonSchema;
use serde::Serialize;

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
