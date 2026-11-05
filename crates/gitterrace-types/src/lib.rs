//! The types every gitterrace crate shares: the values a user sets or reads (config enums, git
//! kinds) and, as they move here, the shapes of the JSON API. One definition each, so a value
//! is spelled, parsed and documented in one place. A type used inside one crate stays there.
//!
//! Dependencies stay light (serde and schemars), so any later binding of these types adds a
//! feature, not a rewrite.

pub mod api;
pub mod card;
pub mod codec;
pub mod config;
pub mod git;
pub mod markup;

#[cfg(test)]
mod registry_tests;

pub use card::{CardLayout, CardTheme};
pub use codec::{ParseError, StringEnum};
pub use config::{ColorScheme, DateStyle, LinkStyle, Period};
pub use git::{ChangeKind, LineKind, RefKind, TreeEntryKind};
pub use markup::{esc, esc_text};

// The codec macro names serde through this path, so a crate using it needs no serde of its own.
#[doc(hidden)]
pub use schemars as __schemars;
#[doc(hidden)]
pub use serde as __serde;
