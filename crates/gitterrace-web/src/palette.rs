//! The design palette as Rust constants, for server-side SVG and shareable cards.
//!
//! `static/tokens.css` is the source of truth; a test fails if the two drift.
//! The module has no dependencies so `gitterrace-cards` can mount it with
//! `#[path = "../../gitterrace-web/src/palette.rs"] mod palette;` instead of
//! depending on the web crate.

/// One theme's colors as `#rrggbb` strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Page background.
    pub bg: &'static str,
    /// Raised surface (cards, header).
    pub surface: &'static str,
    /// Subtle fill (table heads, code blocks).
    pub soft: &'static str,
    /// Hairlines and borders.
    pub line: &'static str,
    /// Tertiary text, axis ticks.
    pub faint: &'static str,
    /// Secondary text.
    pub muted: &'static str,
    /// Primary text.
    pub fg: &'static str,
    /// The single brand accent.
    pub accent: &'static str,
    /// Lines added.
    pub add: &'static str,
    /// Lines deleted.
    pub del: &'static str,
    /// Warnings.
    pub warn: &'static str,
    /// Neutral information.
    pub info: &'static str,
    /// Heatmap ramp, index 0 means no data.
    pub seq: [&'static str; 5],
    /// Categorical series; the last entry is reserved for "other".
    pub cat: [&'static str; 8],
}

/// Light theme.
pub const LIGHT: Palette = Palette {
    bg: "#fafaf8",
    surface: "#ffffff",
    soft: "#f4f3ef",
    line: "#e2e0d9",
    faint: "#9c998f",
    muted: "#66645d",
    fg: "#1c1b19",
    accent: "#0f766e",
    add: "#1a7f37",
    del: "#c0262d",
    warn: "#9a6700",
    info: "#0969da",
    seq: ["#ecebe6", "#c4e5de", "#7fc6b8", "#2f9c8d", "#0b625b"],
    cat: [
        "#0072b2", "#e69f00", "#009e73", "#cc79a7", "#56b4e9", "#d55e00", "#b59c00", "#a8a59c",
    ],
};

/// Dark theme.
pub const DARK: Palette = Palette {
    bg: "#131519",
    surface: "#1a1d22",
    soft: "#1f2329",
    line: "#2d3138",
    faint: "#6b707a",
    muted: "#9da1a9",
    fg: "#e8e9ec",
    accent: "#3fb8a8",
    add: "#56c271",
    del: "#f0706b",
    warn: "#d29922",
    info: "#4493f8",
    seq: ["#22262c", "#17423d", "#1d6b61", "#2c9a8a", "#62d6c5"],
    cat: [
        "#3d9be0", "#f0b030", "#2bbf8f", "#e08fbf", "#7cc8f0", "#f07a33", "#d8c440", "#5d626b",
    ],
};

#[cfg(test)]
mod tests {
    use super::{DARK, LIGHT, Palette};

    const TOKENS: &str = include_str!("../static/tokens.css");

    /// Value of `--name` within one theme block of tokens.css.
    fn token<'a>(block: &'a str, name: &str) -> &'a str {
        let key = format!("--{name}:");
        let start = block.find(&key).map(|i| i + key.len());
        let start = start.unwrap_or_else(|| panic!("missing token {name}"));
        let rest = &block[start..];
        let end = rest.find([';', '\n', '}']).unwrap_or(rest.len());
        &rest[..end]
    }

    fn check(block: &str, p: &Palette) {
        let pairs = [
            ("n-1", p.bg),
            ("n-0", p.surface),
            ("n-2", p.soft),
            ("n-4", p.line),
            ("n-6", p.faint),
            ("n-7", p.muted),
            ("n-9", p.fg),
            ("accent", p.accent),
            ("add", p.add),
            ("del", p.del),
            ("warn", p.warn),
            ("info", p.info),
        ];
        for (name, want) in pairs {
            assert_eq!(token(block, name), want, "--{name}");
        }
        for (i, want) in p.seq.iter().enumerate() {
            assert_eq!(token(block, &format!("seq-{i}")), *want);
        }
        for (i, want) in p.cat.iter().enumerate() {
            assert_eq!(token(block, &format!("cat-{}", i + 1)), *want);
        }
    }

    #[test]
    fn rust_palette_matches_tokens_css() {
        let dark_at = TOKENS
            .find("@media (prefers-color-scheme:dark)")
            .expect("dark block");
        let forced_at = TOKENS
            .find(":root[data-theme=dark]")
            .expect("forced dark block");
        check(&TOKENS[..dark_at], &LIGHT);
        check(&TOKENS[dark_at..forced_at], &DARK);
        // The forced-dark block must equal the media-query block.
        check(&TOKENS[forced_at..], &DARK);
    }
}
