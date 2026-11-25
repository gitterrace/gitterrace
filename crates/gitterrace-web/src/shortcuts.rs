//! The page shortcuts: one list, `static/keys.json`, that the `?` sheet in `nav.js` renders and
//! the Keys table of `docs/using.md` is generated from.

use serde::Deserialize;

const KEYS_JSON: &str = include_str!("../static/keys.json");

/// One shortcut of the sheet.
#[derive(Debug, Deserialize)]
pub struct Shortcut {
    /// The keys as the sheet shows them: `g c` is a chord, `j / k` are alternatives.
    pub keys: String,
    /// The sheet's line.
    pub help: String,
    /// The docs' wording, set only for the common keys the guide's table names.
    #[serde(default)]
    pub doc: Option<String>,
    /// The feature whose switch hides the key and its line.
    #[serde(default)]
    pub feature: Option<String>,
}

/// Every shortcut, in the order the sheet lists them.
///
/// # Panics
///
/// Never at run time: a test parses the embedded file.
pub fn shortcuts() -> Vec<Shortcut> {
    serde_json::from_str(KEYS_JSON).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::shortcuts;

    #[test]
    fn the_embedded_key_list_parses_and_names_the_page_keys() {
        let all = shortcuts();
        assert!(all.len() >= 10, "keys.json did not parse: {all:?}");
        for k in ["t", "/", "y", "b", "g c", "?", "Esc"] {
            assert!(all.iter().any(|s| s.keys == k), "{k} missing");
        }
    }

    /// Class guard: a key `nav.js` binds is a key the sheet and the guide can list.
    #[test]
    fn every_single_key_nav_js_binds_is_in_the_list() {
        let nav = include_str!("../static/nav.js");
        let all = shortcuts();
        for k in nav.split("k === \"").skip(1).filter_map(|s| s.split('"').next()) {
            // `g` only arms a chord, listed as `g c` and the rest.
            if k.chars().count() != 1 || k == "g" {
                continue;
            }
            let listed = all.iter().any(|s| s.keys.split(" / ").any(|x| x == k));
            assert!(listed, "{k} not in keys.json");
        }
    }

    #[test]
    fn every_chord_starts_with_the_goto_key() {
        for s in shortcuts().iter().filter(|s| s.keys.contains(' ') && !s.keys.contains('/')) {
            assert!(s.keys.starts_with("g "), "{}", s.keys);
        }
    }
}
