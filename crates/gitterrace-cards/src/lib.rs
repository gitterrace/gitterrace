//! The year-in-review SVG card built from cached commit records.
//!
//! Every function is pure: records in, a self-contained SVG string out (inline styles, a system
//! font stack, no external references). Each takes a [`Theme`].

#![allow(
    clippy::needless_raw_string_hashes,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::too_many_arguments,
    clippy::format_push_string
)]

mod palette;
mod stats;
mod svg;
mod wrapped;

pub use palette::Theme;
pub use stats::{WrappedStats, wrapped_stats};
pub use wrapped::{Layout, wrapped_svg};
