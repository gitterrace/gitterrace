//! Shareable SVG cards and badges built from cached commit records.
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

mod badges;
mod palette;
mod repo_card;
mod stats;
mod svg;
mod wrapped;

pub use badges::{Badge, badge_svg};
pub use palette::Theme;
pub use repo_card::repo_card_svg;
pub use stats::{RepoSummary, WrappedStats, repo_summary, wrapped_stats};
pub use wrapped::{Layout, wrapped_svg};
