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
    pub on_accent: &'static str,
    /// Categorical colors; the last is "other".
    pub series: [&'static str; 8],
}

impl Theme {
    /// `dark` or `light` (case-sensitive); anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

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
        bg: c.surfaces_bg.hex,
        bg2: c.surfaces_soft.hex,
        surface: c.surfaces_window.hex,
        fg: c.ink_primary.hex,
        muted: c.ink_secondary.hex,
        faint: c.ink_tertiary.hex,
        line: c.surfaces_line.hex,
        accent: c.accent_ink.hex,
        on_accent,
        series: [
            c.series_1.hex,
            c.series_2.hex,
            c.series_3.hex,
            c.series_4.hex,
            c.series_5.hex,
            c.series_6.hex,
            c.series_7.hex,
            c.series_8.hex,
        ],
    }
}
