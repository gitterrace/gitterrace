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
    pub fg: &'static str,
    pub muted: &'static str,
    pub faint: &'static str,
    pub line: &'static str,
    pub accent: &'static str,
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
            Self::Light => LIGHT.get_or_init(|| build(&KANDEL.light)),
            Self::Dark => DARK.get_or_init(|| build(&KANDEL.dark)),
        }
    }
}

const fn build(c: &Colours) -> Palette {
    Palette {
        bg: c.surfaces_bg.hex,
        bg2: c.surfaces_soft.hex,
        fg: c.ink_primary.hex,
        muted: c.ink_secondary.hex,
        faint: c.ink_tertiary.hex,
        line: c.surfaces_line.hex,
        accent: c.accent_ink.hex,
    }
}
