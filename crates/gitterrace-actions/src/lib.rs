//! User-configured actions: "send this line, hunk or commit to my script". The security
//! boundary of gitterrace's only state-changing path, so it holds no HTTP and no rendering.
//!
//! - [`config`] loads the user's `actions.toml`, never one a repository ships.
//! - [`payload`] builds the v1 JSON from a locator by reading git, never from browser text.
//! - [`runner`] spawns the configured argv with a scrubbed env, limits and a group timeout.
//! - [`send`] checks `[[send]]` URL templates the page fills and opens: no process, no POST.
//! - [`token`] is the per-process secret the browser must echo back.

pub mod config;
pub mod link;
mod numbers;
pub mod payload;
pub mod runner;
pub mod schema;
pub mod send;
pub mod source;
pub mod token;

pub use config::{Action, Config, ConfigError, Cwd, Kind, Output, RESERVED_KEYS, Reloader};
pub use link::{Link, LinkError, styled};
pub use payload::{Lines, Locator, Payload, PayloadError, RunRequest, permalink};
pub use runner::{Limits, Outcome, RunError, Runner};
pub use send::{Part, fill};
pub use source::{SOURCE_PREFIX, Source, parse_source, source_line};
pub use token::{Token, proof as token_proof};
