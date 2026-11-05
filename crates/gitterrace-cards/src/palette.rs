use std::sync::OnceLock;

use gitterrace_types::CardTheme;
use schwarzwald::{KANDEL, Palette as Colours};

pub(crate) struct Palette {
    pub bg: &'static str,
    pub bg2: &'static str,
    pub fg: &'static str,
    pub muted: &'static str,
    pub faint: &'static str,
    pub line: &'static str,
    pub accent: &'static str,
}

/// Colors come from the vendored schwarzwald crate (Kandel).
pub(crate) trait Paint {
    fn palette(self) -> &'static Palette;
}

impl Paint for CardTheme {
    fn palette(self) -> &'static Palette {
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
