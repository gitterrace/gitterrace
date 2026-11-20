//! The values of the `config.toml` settings that pick one of a fixed set. Config holds these
//! types, so a page, the API and the docs read the variant, never compare text.

use crate::string_enum;

const DAY: i64 = 86_400;

string_enum! {
    /// Reporting window, ending at the newest commit (`insights.period` and the `period` of a
    /// URL or API request). The long names are accepted on input and never written.
    ///
    /// Serde still writes the variant name, which `/insights/summary` shows. Moving that to the
    /// canonical text is a visible API change of its own.
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    pub enum Period {
        Week => "1w" | "week",
        Month => "1m" | "month",
        Quarter => "3m" | "quarter",
        Year => "1y" | "year",
        #[default]
        All => "all",
    }
}

impl Period {
    /// Length in days, `None` for all history.
    #[must_use]
    pub const fn days(self) -> Option<i64> {
        match self {
            Self::Week => Some(7),
            Self::Month => Some(30),
            Self::Quarter => Some(90),
            Self::Year => Some(365),
            Self::All => None,
        }
    }

    /// `[from, to)` in epoch seconds with `now` the newest commit. `All` starts at `i64::MIN`.
    /// A superset of the period's local days, kept for the scans that still prune by instant.
    #[must_use]
    pub const fn window(self, now: i64) -> (i64, i64) {
        match self.days() {
            Some(d) => (now.saturating_add(1).saturating_sub(d * DAY), now.saturating_add(1)),
            None => (i64::MIN, now.saturating_add(1)),
        }
    }

    /// The name a period picker shows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Quarter => "Quarter",
            Self::Year => "Year",
            Self::All => "All",
        }
    }
}

string_enum! {
    /// The color scheme a page starts in before the visitor picks one (`display.theme`).
    /// `System` follows the browser.
    #[derive(Default)]
    pub enum ColorScheme [serde] {
        #[default]
        System => "system",
        Light => "light",
        Dark => "dark",
    }
}

string_enum! {
    /// Colors a diff marks additions and removals with (`display.diff_palette`).
    #[derive(Default)]
    pub enum DiffPalette [serde] {
        #[default]
        Default => "default",
        Colorblind => "colorblind",
    }
}

impl DiffPalette {
    /// The `data-diff-palette` value, empty for the default colors.
    #[must_use]
    pub fn attr(self) -> String {
        if self == Self::Default { String::new() } else { self.as_str().to_owned() }
    }
}

string_enum! {
    /// How dates read (`display.dates`).
    #[derive(Default)]
    pub enum DateStyle [serde] {
        #[default]
        Relative => "relative",
        Absolute => "absolute",
    }
}

string_enum! {
    /// Which form Copy permalink, Send to, payloads and the MCP hand out (`links.style`).
    #[derive(Default)]
    pub enum LinkStyle [serde] {
        #[default]
        Gitterrace => "gitterrace",
        Go => "go",
    }
}

string_enum! {
    /// A settings file the settings page reads and writes. A request names one of these, never
    /// a path: the server maps it to the file it was started with.
    pub enum SettingsFile [serde] {
        Config => "config",
        Actions => "actions",
    }
}

#[cfg(test)]
mod tests {
    use crate::codec::assert_round_trip;
    use crate::{ColorScheme, DateStyle, DiffPalette, LinkStyle, Period, SettingsFile, StringEnum};

    #[test]
    fn every_config_value_round_trips() {
        assert_round_trip::<Period>();
        assert_round_trip::<ColorScheme>();
        assert_round_trip::<DateStyle>();
        assert_round_trip::<DiffPalette>();
        assert_round_trip::<LinkStyle>();
        assert_round_trip::<SettingsFile>();
    }

    #[test]
    fn the_accepted_values_read_as_the_config_messages_name_them() {
        assert_eq!(Period::expected(), "1w, 1m, 3m, 1y or all");
        assert_eq!(ColorScheme::expected(), "system, light or dark");
        assert_eq!(DateStyle::expected(), "relative or absolute");
        assert_eq!(LinkStyle::expected(), "gitterrace or go");
    }

    #[test]
    fn long_period_names_parse_but_are_never_written() {
        assert_eq!("quarter".parse(), Ok(Period::Quarter));
        assert_eq!(Period::Quarter.to_string(), "3m");
    }

    #[test]
    fn periods_still_serialize_as_variant_names() {
        let json = serde_json::to_string(&Period::All).unwrap_or_default();
        assert_eq!(json, "\"All\"");
    }
}
