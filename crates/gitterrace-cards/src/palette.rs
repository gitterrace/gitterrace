use crate::design;

/// Light or dark rendering. Colors come from the shared gitterrace design palette.
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

impl Theme {
    pub(crate) fn palette(self) -> Palette {
        let (d, on_accent) = match self {
            Theme::Light => (design::LIGHT, "#ffffff"),
            Theme::Dark => (design::DARK, "#0c1513"),
        };
        Palette {
            bg: d.bg,
            bg2: d.soft,
            surface: d.surface,
            fg: d.fg,
            muted: d.muted,
            faint: d.faint,
            line: d.line,
            accent: d.accent,
            accent_fg: d.accent,
            on_accent,
            series: d.cat,
        }
    }
}
