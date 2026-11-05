//! Values of the year card.

use crate::string_enum;

string_enum! {
    /// Light or dark rendering of the year card (`git terrace wrapped --theme`).
    #[derive(Default)]
    pub enum CardTheme {
        #[default]
        Light => "light",
        Dark => "dark",
    }
}

string_enum! {
    /// The year card's shape (`git terrace wrapped --layout`).
    #[derive(Default)]
    pub enum CardLayout {
        /// 1200x630, for link previews and wide feeds.
        #[default]
        Landscape => "landscape",
        /// 1080x1350, for stories and phone feeds.
        Portrait => "portrait",
    }
}

impl CardLayout {
    /// Width and height in pixels.
    #[must_use]
    pub const fn size(self) -> (u32, u32) {
        match self {
            Self::Landscape => (1200, 630),
            Self::Portrait => (1080, 1350),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::codec::assert_round_trip;
    use crate::{CardLayout, CardTheme};

    #[test]
    fn card_values_round_trip() {
        assert_round_trip::<CardTheme>();
        assert_round_trip::<CardLayout>();
    }
}
