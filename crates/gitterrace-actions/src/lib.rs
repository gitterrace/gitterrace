//! User-configured actions: "send this line, hunk or commit to my script". The security
//! boundary of gitterrace's only state-changing path, so it holds no HTTP and no rendering.
//!
//! - [`config`] loads the user's `actions.toml`, never one a repository ships.
//! - [`payload`] builds the v1 JSON from a locator by reading git, never from browser text.
//! - [`runner`] spawns the configured argv with a scrubbed env, limits and a group timeout.
//! - [`token`] is the per-process secret the browser must echo back.

pub mod config;
pub mod payload;
pub mod runner;
pub mod schema;
pub mod token;

pub use config::{Action, Config, ConfigError, Cwd, Kind, Output, Reloader};
pub use payload::{Lines, Locator, Payload, PayloadError, RunRequest, permalink};
pub use runner::{Limits, Outcome, RunError, Runner};
pub use token::Token;
