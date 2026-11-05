//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move.

use schemars::JsonSchema;
use serde::Serialize;

/// Who did something and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Person {
    pub email: String,
    pub name: String,
    /// Offset from UTC in seconds.
    pub offset: i32,
    /// Seconds since the Unix epoch.
    pub time: i64,
}
