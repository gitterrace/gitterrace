//! The year-in-review SVG card built from the insights cache.
//!
//! Every function is pure: a source in, a self-contained SVG string out (inline styles, a system
//! font stack, no external references). Each takes a [`CardTheme`].

#![expect(
    clippy::needless_raw_string_hashes,
    clippy::many_single_char_names,
    clippy::too_many_arguments,
    clippy::format_push_string,
    reason = "SVG templates keep their quoting uniform; geometry and colour math reads best with x, y, w, h; paired chart variables read best side by side; a flat chart parameter list, a struct adds nothing; template assembly reads best with push_str(&format!)"
)]

mod palette;
mod stats;
mod svg;
mod wrapped;

pub use gitterrace_types::{CardLayout, CardTheme};
pub use stats::{WrappedStats, wrapped_stats};
pub use wrapped::wrapped_svg;
