use std::sync::OnceLock;

use schwarzwald::{KANDEL, Palette as Colours};

/// Light or dark rendering. Colors come from the vendored schwarzwald crate (Kandel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

pub(crate) struct Palette {
    pub bg: &'static str,
    pub bg2: &'static str,
    pub surface: &'static str,
    pub fg: &'static str,
    pub muted: &'static str,
    pub faint: &'static str,
    pub line: &'static str,
    pub accent: &'static str,
    pub accent_fg: &'static str,
    pub on_accent: &'static str,
    /// Categorical colors; the last is "other".
    pub series: [&'static str; 8],
}

/// Card SVG carries lowercase hex; the crate ships uppercase. Each of the two
/// palettes is lowercased once and kept for the life of the process.
fn lower(hex: &str) -> &'static str {
    Box::leak(hex.to_ascii_lowercase().into_boxed_str())
}

impl Theme {
    pub(crate) fn palette(self) -> &'static Palette {
        static LIGHT: OnceLock<Palette> = OnceLock::new();
        static DARK: OnceLock<Palette> = OnceLock::new();
        match self {
            Theme::Light => LIGHT.get_or_init(|| build(&KANDEL.light, "#ffffff")),
            Theme::Dark => DARK.get_or_init(|| build(&KANDEL.dark, "#0c1513")),
        }
    }
}

fn build(c: &Colours, on_accent: &'static str) -> Palette {
    Palette {
        bg: lower(c.surfaces_bg.hex),
        bg2: lower(c.surfaces_soft.hex),
        surface: lower(c.surfaces_window.hex),
        fg: lower(c.ink_primary.hex),
        muted: lower(c.ink_secondary.hex),
        faint: lower(c.ink_tertiary.hex),
        line: lower(c.surfaces_line.hex),
        accent: lower(c.accent_ink.hex),
        accent_fg: lower(c.accent_ink.hex),
        on_accent,
        series: [
            lower(c.series_1.hex),
            lower(c.series_2.hex),
            lower(c.series_3.hex),
            lower(c.series_4.hex),
            lower(c.series_5.hex),
            lower(c.series_6.hex),
            lower(c.series_7.hex),
            lower(c.series_8.hex),
        ],
    }
}
