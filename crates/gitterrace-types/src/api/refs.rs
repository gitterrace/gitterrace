//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move.

use schemars::JsonSchema;
use serde::Serialize;

use crate::git::RefKind;

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
