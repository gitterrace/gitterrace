//! Fields are declared in alphabetical order: the JSON this replaces was a `serde_json` map, which
//! sorts keys, and the bytes must not move.

use schemars::JsonSchema;
use serde::Serialize;

/// One query parameter of an endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ParamDoc {
    pub description: String,
    pub name: String,
    pub required: bool,
    pub ty: String,
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
