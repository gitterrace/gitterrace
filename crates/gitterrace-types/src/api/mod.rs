//! The shapes of the JSON API outside the insights: one response struct per endpoint, shared by
//! the HTTP routes, the MCP tools and the generated `OpenAPI` document.
//!
//! Every struct declares its fields in alphabetical order: the JSON these replace was a
//! `serde_json` map, which sorts keys, and the bytes must not move. Never reorder fields.

mod blame;
mod blob;
mod commit;
mod contributors;
mod index;
mod person;
mod refs;
mod repo;
mod tree;

#[cfg(test)]
mod tests;

pub use blame::{BlameItem, BlameResponse};
pub use blob::BlobResponse;
pub use commit::{CommitResponse, DiffHunk, DiffLine, FileItem, LogCommit, LogResponse};
pub use contributors::{ContributorItem, ContributorsResponse};
pub use index::{EndpointDoc, IndexResponse, InsightList, InsightRow, ParamDoc};
pub use person::Person;
pub use refs::{RefItem, RefsResponse};
pub use repo::{RepoResponse, RepoTotals};
pub use tree::{TreeItem, TreeResponse};
