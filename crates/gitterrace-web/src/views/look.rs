//! How a diff looks, from the `display.diff_*` settings: the page body carries it as attributes.

use gitterrace_config::Display;

/// What the base template writes on `<body>` for diffs.
pub(crate) struct DiffLook {
    /// `data-diff-palette` value, empty for the default colors.
    pub(crate) palette: String,
    /// Wrap long lines.
    pub(crate) wrap: bool,
    /// Tab width in characters, `data-diff-tab` when it is not the default 4.
    pub(crate) tab: u64,
    /// Code font size in pixels, `data-diff-font` when it is not the default 13.
    pub(crate) font: u64,
}

impl DiffLook {
    pub(crate) fn of(d: &Display) -> Self {
        Self {
            palette: d.diff_palette.attr(),
            wrap: d.has_diff_wrap,
            tab: d.diff_tab_chars,
            font: d.diff_font_px,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_look_follows_the_display_settings() {
        let mut d = Display::default();
        let l = DiffLook::of(&d);
        assert!(l.palette.is_empty() && !l.wrap && l.tab == 4 && l.font == 13);
        d.diff_tab_chars = 8;
        d.has_diff_wrap = true;
        let l = DiffLook::of(&d);
        assert!(l.wrap && l.tab == 8);
    }
}
